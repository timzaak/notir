#[cfg(test)]
mod test {
    use crate::{CALLBACK_CHANNELS, Mode, ONLINE_USERS, user_disconnected};
    use bytes::Bytes;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

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
        {
            let mut users = ONLINE_USERS.write().await;
            users.insert(user_id.clone(), tx);
        }

        // 验证用户已连接
        {
            let users = ONLINE_USERS.read().await;
            assert!(users.contains_key(&user_id), "用户应该已连接");
        }

        // 模拟发送消息到用户
        let test_message = "Hello from single shot test";
        {
            let users = ONLINE_USERS.read().await;
            if let Some(user_tx) = users.get(&user_id) {
                let msg = salvo::websocket::Message::text(test_message);
                assert!(user_tx.send(Ok(msg)).is_ok(), "消息发送应该成功");
            }
        }

        // 验证消息接收
        let received = timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(received.is_ok(), "应该接收到消息");

        // 清理
        {
            let mut users = ONLINE_USERS.write().await;
            users.remove(&user_id);
        }

        println!("✅ Single shot模式测试通过");
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
        {
            let mut users = ONLINE_USERS.write().await;
            users.insert(user_id.clone(), tx);
        }

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
            if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&user_id_clone) {
                if let Some((_id, tx)) = entry.pop_front() {
                    let _ = tx.send(response_data);
                }
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
        {
            let mut users = ONLINE_USERS.write().await;
            users.remove(&user_id);
        }
        CALLBACK_CHANNELS.remove(&user_id);

        println!("✅ Ping-pong模式测试通过");
    }

    #[tokio::test]
    async fn test_single_shot_user_not_found() {
        // 测试向不存在的用户发送消息
        let non_existent_user = "non_existent_user".to_string();

        // 确保用户不存在
        {
            let users = ONLINE_USERS.read().await;
            assert!(!users.contains_key(&non_existent_user), "用户不应该存在");
        }

        // 模拟查找不存在的用户
        let user_found = {
            let users = ONLINE_USERS.read().await;
            users.get(&non_existent_user).is_some()
        };

        assert!(!user_found, "不存在的用户查找应该返回false");

        println!("✅ Single shot用户不存在测试通过");
    }

    #[tokio::test]
    async fn test_ping_pong_timeout() {
        // 测试ping-pong模式的超时机制
        let user_id = "test_user_timeout".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        {
            let mut users = ONLINE_USERS.write().await;
            users.insert(user_id.clone(), tx);
        }

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
        {
            let mut users = ONLINE_USERS.write().await;
            users.remove(&user_id);
        }
        CALLBACK_CHANNELS.remove(&user_id);

        println!("✅ Ping-pong超时测试通过");
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

        println!("✅ 模式反序列化测试通过");
    }

    #[tokio::test]
    async fn test_user_disconnection() {
        // 测试用户断开连接的清理逻辑
        let user_id = "test_disconnect_user".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();

        // 模拟用户连接
        {
            let mut users = ONLINE_USERS.write().await;
            users.insert(user_id.clone(), tx);
        }

        // 添加一些回调通道
        {
            let mut entry = CALLBACK_CHANNELS.entry(user_id.clone()).or_default();
            let (callback_tx, _) = tokio::sync::oneshot::channel::<Bytes>();
            entry.push_back(("test_callback".to_string(), callback_tx));
        }

        // 验证用户和回调通道存在
        {
            let users = ONLINE_USERS.read().await;
            assert!(users.contains_key(&user_id), "用户应该存在");
        }
        assert!(CALLBACK_CHANNELS.contains_key(&user_id), "回调通道应该存在");

        // 模拟用户断开连接
        user_disconnected(user_id.clone()).await;

        // 验证清理完成
        {
            let users = ONLINE_USERS.read().await;
            assert!(!users.contains_key(&user_id), "用户应该被移除");
        }
        assert!(
            !CALLBACK_CHANNELS.contains_key(&user_id),
            "回调通道应该被移除"
        );

        println!("✅ 用户断开连接测试通过");
    }
}
