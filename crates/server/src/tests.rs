#[cfg(test)]
mod test {
    use crate::broadcast::{BROADCAST_USERS, Connection};
    use crate::files::{
        self, ACTIVE_TRANSFERS, FILE_OFFERS, TransferEvent, handle_client_op, holder_disconnected,
        route_chunk, try_start_transfer,
    };
    use crate::single::{CALLBACK_CHANNELS, Mode, ONLINE_USERS, user_disconnected};
    use bytes::Bytes;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    /// 注册一个假广播连接，返回其消息接收端（用于观察控制消息）
    async fn register_test_connection(
        room_id: &str,
        conn_id: u64,
    ) -> mpsc::UnboundedReceiver<Result<salvo::websocket::Message, salvo::Error>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut users_map = BROADCAST_USERS.write().await;
        users_map
            .entry(room_id.to_string())
            .or_default()
            .push(Connection {
                connection_id: conn_id,
                sender: tx,
            });
        rx
    }

    async fn cleanup_room(room_id: &str) {
        FILE_OFFERS.retain(|_, offer| offer.room_id != room_id);
        BROADCAST_USERS.write().await.remove(room_id);
    }

    async fn next_control(
        rx: &mut mpsc::UnboundedReceiver<Result<salvo::websocket::Message, salvo::Error>>,
    ) -> serde_json::Value {
        let msg = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("应该收到控制消息")
            .expect("通道不应关闭")
            .expect("消息不应出错");
        assert!(msg.is_binary(), "控制消息应为二进制帧");
        serde_json::from_slice(msg.as_bytes()).expect("控制消息应为合法 JSON")
    }

    #[tokio::test]
    async fn test_single_shot_mode() {
        // 测试 Mode::Shot 的反序列化
        let mode_json = r#""shot""#;
        let mode: Mode = serde_json::from_str(mode_json).unwrap();
        assert!(matches!(mode, Mode::Shot));

        // 测试用户连接和消息发送逻辑
        let user_id = "test_user_shot".to_string();
        let (tx, mut rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        let conn_id = "test_conn".to_string();
        ONLINE_USERS
            .entry(user_id.clone())
            .or_default()
            .insert(conn_id.clone(), tx);

        // 验证用户已连接
        assert!(ONLINE_USERS.contains_key(&user_id), "用户应该已连接");

        // 模拟发送消息到用户
        let test_message = "Hello from single shot test";
        if let Some(user_conns) = ONLINE_USERS.get(&user_id)
            && let Some(conn) = user_conns.get(&conn_id)
        {
            let msg = salvo::websocket::Message::text(test_message);
            assert!(conn.send(Ok(msg)).is_ok(), "消息发送应该成功");
        }

        // 验证消息接收
        let received = timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(received.is_ok(), "应该接收到消息");

        // 清理
        ONLINE_USERS.remove(&user_id);
    }

    #[tokio::test]
    async fn test_ping_pong_mode() {
        // 测试 Mode::PingPong 的反序列化
        let mode_json = r#""ping_pong""#;
        let mode: Mode = serde_json::from_str(mode_json).unwrap();
        assert!(matches!(mode, Mode::PingPong));

        // 测试ping-pong模式的回调机制
        let user_id = "test_user_pingpong".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        let conn_id = "test_conn".to_string();
        ONLINE_USERS
            .entry(user_id.clone())
            .or_default()
            .insert(conn_id.clone(), tx);

        // 创建回调通道
        let (callback_tx, callback_rx) = tokio::sync::oneshot::channel();
        let callback_id = nanoid::nanoid!();

        // 添加回调到队列
        {
            let mut entry = CALLBACK_CHANNELS.entry(user_id.clone()).or_default();
            entry.push_back((callback_id.clone(), callback_tx));
        }

        // 验证回调通道已添加
        {
            let entry = CALLBACK_CHANNELS.get(&user_id).unwrap();
            assert_eq!(entry.len(), 1, "应该有一个回调通道");
        }

        // 模拟客户端回复
        let response_data = Bytes::from("Pong response from client");
        let user_id_clone = user_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&user_id_clone)
                && let Some((_id, tx)) = entry.pop_front()
            {
                let _ = tx.send(response_data);
            }
        });

        // 等待回调响应
        let result = timeout(Duration::from_secs(1), callback_rx).await;
        assert!(result.is_ok(), "应该接收到回调响应");

        let response = result.unwrap().unwrap();
        assert_eq!(
            response,
            Bytes::from("Pong response from client"),
            "回调响应内容应该匹配"
        );

        // 清理
        ONLINE_USERS.remove(&user_id);
        CALLBACK_CHANNELS.remove(&user_id);
    }

    #[tokio::test]
    async fn test_single_shot_user_not_found() {
        // 测试向不存在的用户发送消息
        let non_existent_user = "non_existent_user".to_string();

        // 确保用户不存在
        assert!(
            !ONLINE_USERS.contains_key(&non_existent_user),
            "用户不应该存在"
        );

        // 模拟查找不存在的用户
        let user_found = ONLINE_USERS.get(&non_existent_user).is_some();

        assert!(!user_found, "不存在的用户查找应该返回false");
    }

    #[tokio::test]
    async fn test_ping_pong_timeout() {
        // 测试ping-pong模式的超时机制
        let user_id = "test_user_timeout".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        let conn_id = "test_conn".to_string();
        ONLINE_USERS
            .entry(user_id.clone())
            .or_default()
            .insert(conn_id.clone(), tx);

        // 创建回调通道但不回复
        let (callback_tx, callback_rx) = tokio::sync::oneshot::channel::<Bytes>();
        let callback_id = nanoid::nanoid!();

        // 添加回调到队列
        {
            let mut entry = CALLBACK_CHANNELS.entry(user_id.clone()).or_default();
            entry.push_back((callback_id.clone(), callback_tx));
        }

        // 模拟5秒超时
        let result = timeout(Duration::from_millis(100), callback_rx).await; // 使用较短的超时进行测试
        assert!(result.is_err(), "应该发生超时");

        // 验证超时后回调通道被清理
        {
            let mut entry = CALLBACK_CHANNELS.entry(user_id.clone()).or_default();
            entry.retain(|(id, _)| id != &callback_id);
        }

        // 清理
        ONLINE_USERS.remove(&user_id);
        CALLBACK_CHANNELS.remove(&user_id);
    }

    #[tokio::test]
    async fn test_mode_deserialization() {
        // 测试默认模式
        let default_mode = Mode::default();
        assert!(matches!(default_mode, Mode::Shot));

        // 测试从字符串反序列化
        let shot_mode: Mode = serde_json::from_str(r#""shot""#).unwrap();
        assert!(matches!(shot_mode, Mode::Shot));

        let ping_pong_mode: Mode = serde_json::from_str(r#""ping_pong""#).unwrap();
        assert!(matches!(ping_pong_mode, Mode::PingPong));
    }

    #[tokio::test]
    async fn test_user_disconnection() {
        // 测试用户断开连接的清理逻辑
        let user_id = "test_disconnect_user".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        let conn_id = "test_conn".to_string();
        ONLINE_USERS
            .entry(user_id.clone())
            .or_default()
            .insert(conn_id.clone(), tx);

        // 添加一些回调通道
        {
            let mut entry = CALLBACK_CHANNELS.entry(user_id.clone()).or_default();
            let (callback_tx, _) = tokio::sync::oneshot::channel::<Bytes>();
            entry.push_back(("test_callback".to_string(), callback_tx));
        }

        // 验证用户和回调通道存在
        assert!(ONLINE_USERS.contains_key(&user_id), "用户应该存在");
        assert!(CALLBACK_CHANNELS.contains_key(&user_id), "回调通道应该存在");

        // 模拟用户断开连接
        user_disconnected(user_id.clone(), conn_id).await;

        // 验证清理完成
        assert!(!ONLINE_USERS.contains_key(&user_id), "用户应该被移除");
        assert!(
            !CALLBACK_CHANNELS.contains_key(&user_id),
            "回调通道应该被移除"
        );
    }

    // ========== Broadcast 模块测试 ==========

    #[tokio::test]
    async fn test_broadcast_users_pool() {
        // 测试广播用户连接池的基本操作
        let user_id = "test_broadcast_user".to_string();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // 添加用户到广播池
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let connection = Connection {
                connection_id: 1,
                sender: tx,
            };
            users_map
                .entry(user_id.clone())
                .or_default()
                .push(connection);
        }

        // 验证用户已添加
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(users_map.contains_key(&user_id), "用户应该在广播池中");
            assert_eq!(users_map.get(&user_id).unwrap().len(), 1, "应该有一个连接");
        }

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_multiple_connections() {
        // 测试同一用户的多个连接
        let user_id = "test_multi_broadcast_user".to_string();
        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加多个连接到同一用户
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let entry = users_map.entry(user_id.clone()).or_default();
            entry.push(Connection {
                connection_id: 1,
                sender: tx1,
            });
            entry.push(Connection {
                connection_id: 2,
                sender: tx2,
            });
        }

        // 验证多个连接
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(users_map.contains_key(&user_id), "用户应该在广播池中");
            assert_eq!(users_map.get(&user_id).unwrap().len(), 2, "应该有两个连接");
        }

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_message_distribution() {
        // 测试消息分发到多个连接
        let user_id = "test_message_dist_user".to_string();
        let (tx1, mut rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加连接到广播池
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let entry = users_map.entry(user_id.clone()).or_default();
            entry.push(Connection {
                connection_id: 1,
                sender: tx1,
            });
            entry.push(Connection {
                connection_id: 2,
                sender: tx2,
            });
        }

        // 模拟消息分发
        let test_message = salvo::websocket::Message::text("Broadcast test message");
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections {
                    let _ = connection.sender.send(Ok(test_message.clone()));
                }
            }
        }

        // 验证两个连接都收到消息
        let msg1 = timeout(Duration::from_secs(1), rx1.recv()).await;
        let msg2 = timeout(Duration::from_secs(1), rx2.recv()).await;

        assert!(msg1.is_ok(), "第一个连接应该收到消息");
        assert!(msg2.is_ok(), "第二个连接应该收到消息");

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_failed_connection_cleanup() {
        // 测试失败连接的清理机制
        let user_id = "test_cleanup_user".to_string();
        let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加连接到广播池
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let entry = users_map.entry(user_id.clone()).or_default();
            entry.push(Connection {
                connection_id: 1,
                sender: tx1,
            });
            entry.push(Connection {
                connection_id: 2,
                sender: tx2,
            });
        }

        // 关闭第一个接收器，模拟连接断开
        drop(rx1);

        // 模拟发送消息，第一个连接会失败
        let test_message = salvo::websocket::Message::text("Test cleanup message");
        let mut failed_connections = Vec::new();

        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for (index, connection) in connections.iter().enumerate() {
                    if connection.sender.send(Ok(test_message.clone())).is_err() {
                        failed_connections.push(index);
                    }
                }
            }
        }

        // 验证有失败的连接
        assert_eq!(failed_connections.len(), 1, "应该有一个失败的连接");

        // 模拟清理失败的连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            if let Some(connections) = users_map.get_mut(&user_id) {
                for &index in failed_connections.iter().rev() {
                    if index < connections.len() {
                        connections.remove(index);
                    }
                }
            }
        }

        // 验证清理后只剩一个连接
        {
            let users_map = BROADCAST_USERS.read().await;
            assert_eq!(
                users_map.get(&user_id).unwrap().len(),
                1,
                "应该只剩一个连接"
            );
        }

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_empty_user_cleanup() {
        // 测试当用户没有连接时的清理
        let user_id = "test_empty_cleanup_user".to_string();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // 添加连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let connection = Connection {
                connection_id: 1,
                sender: tx,
            };
            users_map
                .entry(user_id.clone())
                .or_default()
                .push(connection);
        }

        // 关闭接收器
        drop(rx);

        // 模拟发送消息失败并清理
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                let mut failed_connections = Vec::new();
                for (index, connection) in connections.iter().enumerate() {
                    if connection
                        .sender
                        .send(Ok(salvo::websocket::Message::text("test")))
                        .is_err()
                    {
                        failed_connections.push(index);
                    }
                }

                drop(users_map);
                let mut users_map = BROADCAST_USERS.write().await;
                if let Some(connections) = users_map.get_mut(&user_id) {
                    for &index in failed_connections.iter().rev() {
                        if index < connections.len() {
                            connections.remove(index);
                        }
                    }
                    // 如果没有连接了，移除整个条目
                    if connections.is_empty() {
                        users_map.remove(&user_id);
                    }
                }
            }
        }

        // 验证用户已被完全移除
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(!users_map.contains_key(&user_id), "空用户应该被移除");
        }
    }

    #[tokio::test]
    async fn test_broadcast_message_types() {
        // 测试不同类型的消息处理
        let user_id = "test_message_types_user".to_string();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // 添加连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let connection = Connection {
                connection_id: 1,
                sender: tx,
            };
            users_map
                .entry(user_id.clone())
                .or_default()
                .push(connection);
        }

        // 测试文本消息
        let text_msg = salvo::websocket::Message::text("Hello World");
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections {
                    let _ = connection.sender.send(Ok(text_msg.clone()));
                }
            }
        }

        let received_text = timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(received_text.is_ok(), "应该收到文本消息");

        // 测试二进制消息
        let binary_msg = salvo::websocket::Message::binary(vec![1, 2, 3, 4]);
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections {
                    let _ = connection.sender.send(Ok(binary_msg.clone()));
                }
            }
        }

        let received_binary = timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(received_binary.is_ok(), "应该收到二进制消息");

        // 测试ping消息
        let ping_msg = salvo::websocket::Message::ping(vec![]);
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections {
                    let _ = connection.sender.send(Ok(ping_msg.clone()));
                }
            }
        }

        let received_ping = timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(received_ping.is_ok(), "应该收到ping消息");

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_concurrent_access() {
        // 测试并发访问广播池
        let user_id = "test_concurrent_user".to_string();
        let mut handles = Vec::new();

        // 启动多个任务并发添加连接
        for i in 0..10 {
            let user_id_clone = user_id.clone();
            let handle = tokio::spawn(async move {
                let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                let mut users_map = BROADCAST_USERS.write().await;
                let connection = Connection {
                    connection_id: i as u64,
                    sender: tx,
                };
                users_map.entry(user_id_clone).or_default().push(connection);
                i
            });
            handles.push(handle);
        }

        // 等待所有任务完成
        for handle in handles {
            handle.await.unwrap();
        }

        // 验证所有连接都已添加
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(users_map.contains_key(&user_id), "用户应该存在");
            assert_eq!(users_map.get(&user_id).unwrap().len(), 10, "应该有10个连接");
        }

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_connection_disconnect_scenario() {
        // 测试具体场景：两个连接，一个断开后，另一个仍能接收消息
        let user_id = "test_disconnect_scenario".to_string();
        let (tx1, mut rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加两个连接到广播池
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let entry = users_map.entry(user_id.clone()).or_default();
            entry.push(Connection {
                connection_id: 1,
                sender: tx1,
            });
            entry.push(Connection {
                connection_id: 2,
                sender: tx2,
            });
        }

        // 验证两个连接都已添加
        {
            let users_map = BROADCAST_USERS.read().await;
            assert_eq!(users_map.get(&user_id).unwrap().len(), 2, "应该有两个连接");
        }

        // 模拟第二个连接断开（关闭接收器）
        drop(rx2);

        // 模拟通过 /broad/pub 发送消息的逻辑
        let test_message = salvo::websocket::Message::text("Test message after disconnect");
        let mut failed_connection_ids = Vec::new();

        // 尝试发送消息给所有连接
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections.iter() {
                    if connection.sender.send(Ok(test_message.clone())).is_err() {
                        failed_connection_ids.push(connection.connection_id);
                        tracing::debug!(
                            "Failed to send message to connection_id: {}",
                            connection.connection_id
                        );
                    }
                }
            }
        }

        // 验证有一个连接失败
        assert_eq!(failed_connection_ids.len(), 1, "应该有一个失败的连接");
        assert_eq!(failed_connection_ids[0], 2, "失败的应该是 connection_id 2");

        // 清理失败的连接（模拟 broadcast_publish 中的清理逻辑）
        {
            let mut users_map = BROADCAST_USERS.write().await;
            if let Some(connections) = users_map.get_mut(&user_id) {
                connections.retain(|conn| !failed_connection_ids.contains(&conn.connection_id));

                // 如果没有连接了，移除整个条目
                if connections.is_empty() {
                    users_map.remove(&user_id);
                }
            }
        }

        // 验证清理后只剩一个连接
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(users_map.contains_key(&user_id), "用户应该仍然存在");
            assert_eq!(
                users_map.get(&user_id).unwrap().len(),
                1,
                "应该只剩一个连接"
            );
            assert_eq!(
                users_map.get(&user_id).unwrap()[0].connection_id,
                1,
                "剩余的应该是 connection_id 1"
            );
        }

        // 验证剩余连接能收到之前发送的消息
        let received_msg = timeout(Duration::from_millis(100), rx1.recv()).await;
        assert!(received_msg.is_ok(), "剩余连接应该能收到消息");

        // 再次发送消息，验证剩余连接仍然工作正常
        let second_message = salvo::websocket::Message::text("Second test message");
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections.iter() {
                    let send_result = connection.sender.send(Ok(second_message.clone()));
                    assert!(send_result.is_ok(), "发送给剩余连接应该成功");
                }
            }
        }

        // 验证第二条消息也能正常接收
        let received_second_msg = timeout(Duration::from_millis(100), rx1.recv()).await;
        assert!(received_second_msg.is_ok(), "剩余连接应该能收到第二条消息");

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user_id);
        }
    }

    #[tokio::test]
    async fn test_broadcast_all_connections_disconnect() {
        // 测试所有连接都断开的场景
        let user_id = "test_all_disconnect".to_string();
        let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加两个连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let entry = users_map.entry(user_id.clone()).or_default();
            entry.push(Connection {
                connection_id: 1,
                sender: tx1,
            });
            entry.push(Connection {
                connection_id: 2,
                sender: tx2,
            });
        }

        // 断开所有连接
        drop(rx1);
        drop(rx2);

        // 尝试发送消息
        let test_message = salvo::websocket::Message::text("Message to disconnected connections");
        let mut failed_connection_ids = Vec::new();

        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user_id) {
                for connection in connections.iter() {
                    if connection.sender.send(Ok(test_message.clone())).is_err() {
                        failed_connection_ids.push(connection.connection_id);
                    }
                }
            }
        }

        // 验证所有连接都失败
        assert_eq!(failed_connection_ids.len(), 2, "所有连接都应该失败");

        // 清理失败的连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            if let Some(connections) = users_map.get_mut(&user_id) {
                connections.retain(|conn| !failed_connection_ids.contains(&conn.connection_id));

                // 如果没有连接了，移除整个条目
                if connections.is_empty() {
                    users_map.remove(&user_id);
                }
            }
        }

        // 验证用户条目已被完全移除
        {
            let users_map = BROADCAST_USERS.read().await;
            assert!(!users_map.contains_key(&user_id), "用户条目应该被完全移除");
        }
    }

    #[tokio::test]
    async fn test_broadcast_partial_disconnect_multiple_users() {
        // 测试多个用户，部分连接断开的复杂场景
        let user1_id = "user1".to_string();
        let user2_id = "user2".to_string();

        let (user1_tx1, mut user1_rx1) = tokio::sync::mpsc::unbounded_channel();
        let (user1_tx2, user1_rx2) = tokio::sync::mpsc::unbounded_channel(); // 这个会断开
        let (user2_tx1, mut user2_rx1) = tokio::sync::mpsc::unbounded_channel();
        let (user2_tx2, mut user2_rx2) = tokio::sync::mpsc::unbounded_channel();

        // 添加连接
        {
            let mut users_map = BROADCAST_USERS.write().await;

            // 用户1的连接
            let user1_entry = users_map.entry(user1_id.clone()).or_default();
            user1_entry.push(Connection {
                connection_id: 1,
                sender: user1_tx1,
            });
            user1_entry.push(Connection {
                connection_id: 2,
                sender: user1_tx2,
            });

            // 用户2的连接
            let user2_entry = users_map.entry(user2_id.clone()).or_default();
            user2_entry.push(Connection {
                connection_id: 3,
                sender: user2_tx1,
            });
            user2_entry.push(Connection {
                connection_id: 4,
                sender: user2_tx2,
            });
        }

        // 断开用户1的第二个连接
        drop(user1_rx2);

        // 给用户1发送消息
        let message_for_user1 = salvo::websocket::Message::text("Message for user1");
        let mut failed_connection_ids = Vec::new();

        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user1_id) {
                for connection in connections.iter() {
                    if connection
                        .sender
                        .send(Ok(message_for_user1.clone()))
                        .is_err()
                    {
                        failed_connection_ids.push(connection.connection_id);
                    }
                }
            }
        }

        // 清理用户1的失败连接
        {
            let mut users_map = BROADCAST_USERS.write().await;
            if let Some(connections) = users_map.get_mut(&user1_id) {
                connections.retain(|conn| !failed_connection_ids.contains(&conn.connection_id));
            }
        }

        // 验证用户1只剩一个连接，用户2仍有两个连接
        {
            let users_map = BROADCAST_USERS.read().await;
            assert_eq!(
                users_map.get(&user1_id).unwrap().len(),
                1,
                "用户1应该只剩一个连接"
            );
            assert_eq!(
                users_map.get(&user2_id).unwrap().len(),
                2,
                "用户2应该仍有两个连接"
            );
        }

        // 验证用户1的剩余连接能收到消息
        let received_msg = timeout(Duration::from_millis(100), user1_rx1.recv()).await;
        assert!(received_msg.is_ok(), "用户1的剩余连接应该能收到消息");

        // 给用户2发送消息，验证不受影响
        let message_for_user2 = salvo::websocket::Message::text("Message for user2");
        {
            let users_map = BROADCAST_USERS.read().await;
            if let Some(connections) = users_map.get(&user2_id) {
                for connection in connections.iter() {
                    let send_result = connection.sender.send(Ok(message_for_user2.clone()));
                    assert!(send_result.is_ok(), "用户2的连接发送应该成功");
                }
            }
        }

        // 验证用户2的两个连接都能收到消息
        let user2_msg1 = timeout(Duration::from_millis(100), user2_rx1.recv()).await;
        let user2_msg2 = timeout(Duration::from_millis(100), user2_rx2.recv()).await;
        assert!(user2_msg1.is_ok(), "用户2的第一个连接应该能收到消息");
        assert!(user2_msg2.is_ok(), "用户2的第二个连接应该能收到消息");

        // 清理
        {
            let mut users_map = BROADCAST_USERS.write().await;
            users_map.remove(&user1_id);
            users_map.remove(&user2_id);
        }
    }

    // ========== File transfer 模块测试 ==========

    #[tokio::test]
    async fn test_file_offer_registers_and_replies() {
        let room = "ft_room_offer";
        let conn = 9101u64;
        let mut rx = register_test_connection(room, conn).await;

        handle_client_op(
            room,
            conn,
            r#"{"op":"offer","name":"a.png","size":123,"mime":"image/png","offerId":"o1"}"#,
        )
        .await;

        // offer 已登记
        let offers: Vec<(String, String, u64)> = FILE_OFFERS
            .iter()
            .filter(|entry| entry.value().conn_id == conn)
            .map(|entry| {
                (
                    entry.key().clone(),
                    entry.value().name.clone(),
                    entry.value().size,
                )
            })
            .collect();
        assert_eq!(offers.len(), 1, "应该注册了一个 offer");
        assert_eq!(offers[0].1, "a.png");
        assert_eq!(offers[0].2, 123);

        // offer_ok 控制消息（binary JSON）送达连接
        let ctrl = next_control(&mut rx).await;
        assert_eq!(ctrl["op"], "offer_ok");
        assert_eq!(ctrl["fileId"], offers[0].0);
        assert_eq!(ctrl["offerId"], "o1");

        cleanup_room(room).await;
    }

    #[tokio::test]
    async fn test_non_file_text_ignored() {
        let room = "ft_room_ignore";
        let conn = 9102u64;
        let mut rx = register_test_connection(room, conn).await;

        // 普通剪贴板文本不应注册 offer，也不应回控制消息
        handle_client_op(room, conn, "hello clipboard").await;
        handle_client_op(room, conn, "{\"op\":\"bogus\"}").await;

        assert!(
            FILE_OFFERS
                .iter()
                .all(|entry| entry.value().conn_id != conn),
            "不应注册任何 offer"
        );
        assert!(
            timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "不应收到任何控制消息"
        );

        cleanup_room(room).await;
    }

    #[tokio::test]
    async fn test_chunk_routing_and_done() {
        let conn = 9103u64;
        let mut rx = try_start_transfer(conn).expect("传输槽应为空闲");

        route_chunk(conn, Bytes::from_static(b"abc")).await;
        route_chunk(conn, Bytes::from_static(b"def")).await;
        handle_client_op("ft_room_chunk", conn, r#"{"op":"done"}"#).await;

        match timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(TransferEvent::Chunk(bytes))) => assert_eq!(bytes, Bytes::from_static(b"abc")),
            other => panic!("第一个分块异常: {other:?}"),
        }
        match timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(TransferEvent::Chunk(bytes))) => assert_eq!(bytes, Bytes::from_static(b"def")),
            other => panic!("第二个分块异常: {other:?}"),
        }
        match timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(TransferEvent::Done)) => {}
            other => panic!("应收到 Done 事件: {other:?}"),
        }
        assert!(
            !ACTIVE_TRANSFERS.contains_key(&conn),
            "done 后传输槽应被释放"
        );
    }

    #[tokio::test]
    async fn test_transfer_slot_exclusivity() {
        let conn = 9104u64;
        let _rx = try_start_transfer(conn).expect("第一次占用应成功");
        assert!(try_start_transfer(conn).is_none(), "同连接第二次占用应失败");
        ACTIVE_TRANSFERS.remove(&conn);
        assert!(try_start_transfer(conn).is_some(), "释放后应可再次占用");
        ACTIVE_TRANSFERS.remove(&conn);
    }

    #[tokio::test]
    async fn test_holder_disconnected_cleans_up() {
        let room = "ft_room_disc";
        let conn = 9105u64;
        let mut rx = register_test_connection(room, conn).await;

        handle_client_op(room, conn, r#"{"op":"offer","name":"x.bin","size":10}"#).await;
        let _ = next_control(&mut rx).await; // 消费 offer_ok

        let mut transfer_rx = try_start_transfer(conn).expect("传输槽应为空闲");

        holder_disconnected(room, conn).await;

        assert!(
            FILE_OFFERS
                .iter()
                .all(|entry| entry.value().conn_id != conn),
            "断开后 offer 应被清除"
        );
        match timeout(Duration::from_secs(1), transfer_rx.recv()).await {
            Ok(Some(TransferEvent::Aborted)) => {}
            other => panic!("断开后应收到 Aborted 事件: {other:?}"),
        }

        cleanup_room(room).await;
    }

    #[tokio::test]
    async fn test_offer_count_cap() {
        let room = "ft_room_cap";
        let conn = 9106u64;
        let mut rx = register_test_connection(room, conn).await;

        for i in 0..32 {
            handle_client_op(
                room,
                conn,
                &format!(r#"{{"op":"offer","name":"f{i}.bin","size":1}}"#),
            )
            .await;
        }
        let count = FILE_OFFERS
            .iter()
            .filter(|entry| entry.value().conn_id == conn)
            .count();
        assert_eq!(count, 32, "应恰好注册 32 个 offer");

        // 第 33 个应被拒绝并返回 error 控制消息
        handle_client_op(
            room,
            conn,
            r#"{"op":"offer","name":"overflow.bin","size":1}"#,
        )
        .await;
        let count = FILE_OFFERS
            .iter()
            .filter(|entry| entry.value().conn_id == conn)
            .count();
        assert_eq!(count, 32, "不应注册第 33 个 offer");

        // 依次消费全部控制消息，最后一条应为 error
        let mut last_ctrl = serde_json::Value::Null;
        let mut got_any = false;
        while let Ok(Some(Ok(msg))) = timeout(Duration::from_millis(50), rx.recv()).await {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(msg.as_bytes()) {
                last_ctrl = value;
                got_any = true;
            }
        }
        assert!(got_any, "应该收到过控制消息");
        assert_eq!(last_ctrl["op"], "error", "最后一条控制消息应为 error");

        cleanup_room(room).await;
    }

    #[test]
    fn test_percent_encode() {
        assert_eq!(files::percent_encode("a b.png"), "a%20b.png");
        assert_eq!(files::percent_encode("photo~1-_.A.png"), "photo~1-_.A.png");
        assert_eq!(files::percent_encode("文档.zip"), "%E6%96%87%E6%A1%A3.zip");
        assert_eq!(files::percent_encode("q\"x"), "q%22x");
    }
}
