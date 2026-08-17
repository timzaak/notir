import { useCallback, useEffect, useRef, useState } from 'react';
import { WebSocketManager, type WebSocketConfig } from '../utils/WebSocketManager';

const clipboardWsConfig: WebSocketConfig = {
  enableReconnect: true,
  reconnectInterval: 3000,
  maxReconnectAttempts: 20,
  mode: 'broad',
};

const FILE_CHUNK_SIZE = 256 * 1024;
const WS_SEND_BUFFER_LIMIT = 4 * 1024 * 1024;
const MAX_FILE_CARDS = 20;

type FileCardStatus = 'waiting' | 'sending' | 'sent' | 'download-started' | 'unavailable' | 'error';

interface FileCard {
  fileId: string;
  name: string;
  size: number;
  mime: string;
  mine: boolean;
  status: FileCardStatus;
  sentBytes: number;
}

interface PendingOffer {
  file: File;
  replaceFileId?: string;
}

// 服务器经二进制帧发来的控制消息
type ServerControl =
  | { op: 'offer_ok'; fileId: string; offerId?: string }
  | { op: 'pull'; fileId: string }
  | { op: 'cancel' }
  | { op: 'error'; message?: string }
  | { type: 'notir-file'; fileId: string; name: string; size: number; mime?: string };

const formatSyncTime = () => new Date().toLocaleTimeString();

const formatBytes = (n: number): string => {
  if (n < 1024) {
    return `${n} B`;
  }
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = n;
  let unit = -1;
  do {
    value /= 1024;
    unit += 1;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[unit]}`;
};

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

function ClipboardPage() {
  const params = new URLSearchParams(window.location.search);
  const id = params.get('id')?.trim() || '';

  const [clipboardId, setClipboardId] = useState(id);
  const [content, setContent] = useState('');
  const [statusMessage, setStatusMessage] = useState('Checking for ID in URL...');
  const [lastSyncTime, setLastSyncTime] = useState('');
  const [copyStatus, setCopyStatus] = useState('');
  const [fileCards, setFileCards] = useState<FileCard[]>([]);

  const wsManager = useRef<WebSocketManager | null>(null);
  const publishTimer = useRef<number | null>(null);
  const isApplyingRemoteMessage = useRef(false);
  const lastPublishedContent = useRef<string | null>(null);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const localFiles = useRef<Map<string, File>>(new Map());
  const pendingOffers = useRef<Map<string, PendingOffer>>(new Map());
  const sendCancelled = useRef(false);

  const addFileCard = useCallback((card: FileCard) => {
    setFileCards((prev) => {
      if (prev.some((existing) => existing.fileId === card.fileId)) {
        return prev;
      }
      return [card, ...prev].slice(0, MAX_FILE_CARDS);
    });
  }, []);

  const updateFileCard = useCallback((fileId: string, patch: Partial<FileCard>) => {
    setFileCards((prev) => prev.map((card) => (card.fileId === fileId ? { ...card, ...patch } : card)));
  }, []);

  const removeFileCard = useCallback((fileId: string) => {
    localFiles.current.delete(fileId);
    setFileCards((prev) => prev.filter((card) => card.fileId !== fileId));
  }, []);

  const announceFile = useCallback(
    async (card: FileCard) => {
      try {
        const response = await fetch(`/broad/pub?id=${encodeURIComponent(id)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/octet-stream' },
          body: JSON.stringify({
            type: 'notir-file',
            fileId: card.fileId,
            name: card.name,
            size: card.size,
            mime: card.mime,
          }),
        });
        if (!response.ok) {
          throw new Error(`Announce failed with status ${response.status}`);
        }
      } catch (error) {
        console.error('Failed to announce file:', error);
        updateFileCard(card.fileId, { status: 'error' });
      }
    },
    [id, updateFileCard],
  );

  const sendOfferOp = useCallback((file: File, replaceFileId?: string) => {
    const manager = wsManager.current;
    if (!manager || manager.readyState !== WebSocket.OPEN) {
      setStatusMessage('Error: cannot offer file while disconnected.');
      return;
    }
    const offerId = `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    pendingOffers.current.set(offerId, { file, replaceFileId });
    manager.send(
      JSON.stringify({
        op: 'offer',
        offerId,
        name: file.name,
        size: file.size,
        mime: file.type || 'application/octet-stream',
      }),
    );
  }, []);

  // 收到 pull 指令后分块流式发送文件
  const startFileSend = useCallback(
    async (fileId: string) => {
      const manager = wsManager.current;
      const file = localFiles.current.get(fileId);
      if (!manager || !file) {
        manager?.send(JSON.stringify({ op: 'abort' }));
        return;
      }

      updateFileCard(fileId, { status: 'sending', sentBytes: 0 });
      try {
        let offset = 0;
        while (offset < file.size) {
          if (sendCancelled.current) {
            break;
          }
          // 浏览器发送缓冲堆积时暂停读取，避免大文件撑爆内存
          while (manager.bufferedAmount > WS_SEND_BUFFER_LIMIT && !sendCancelled.current) {
            await sleep(50);
          }
          const buffer = await file.slice(offset, offset + FILE_CHUNK_SIZE).arrayBuffer();
          manager.send(buffer);
          offset += buffer.byteLength;
          updateFileCard(fileId, { sentBytes: offset });
        }
        const cancelled = sendCancelled.current;
        sendCancelled.current = false;
        manager.send(JSON.stringify({ op: cancelled ? 'abort' : 'done' }));
        updateFileCard(fileId, {
          status: cancelled ? 'waiting' : 'sent',
          sentBytes: cancelled ? 0 : file.size,
        });
      } catch (error) {
        console.error('Failed to send file:', error);
        manager.send(JSON.stringify({ op: 'abort' }));
        updateFileCard(fileId, { status: 'error', sentBytes: 0 });
      }
    },
    [updateFileCard],
  );

  const handleControl = useCallback(
    (control: ServerControl) => {
      if ('op' in control) {
        if (control.op === 'offer_ok') {
          const offerId = control.offerId ?? '';
          const pending = pendingOffers.current.get(offerId);
          pendingOffers.current.delete(offerId);
          if (!pending) {
            return;
          }
          const { file, replaceFileId } = pending;
          if (replaceFileId) {
            localFiles.current.delete(replaceFileId);
            setFileCards((prev) => prev.filter((card) => card.fileId !== replaceFileId));
          }
          const mime = file.type || 'application/octet-stream';
          localFiles.current.set(control.fileId, file);
          addFileCard({
            fileId: control.fileId,
            name: file.name,
            size: file.size,
            mime,
            mine: true,
            status: 'waiting',
            sentBytes: 0,
          });
          // 文件本身仍留在本地，只广播元数据通告
          void announceFile({
            fileId: control.fileId,
            name: file.name,
            size: file.size,
            mime,
            mine: true,
            status: 'waiting',
            sentBytes: 0,
          });
        } else if (control.op === 'pull') {
          void startFileSend(control.fileId);
        } else if (control.op === 'cancel') {
          sendCancelled.current = true;
        }
        return;
      }

      if (control.type === 'notir-file') {
        addFileCard({
          fileId: control.fileId,
          name: control.name,
          size: control.size,
          mime: control.mime || 'application/octet-stream',
          mine: false,
          status: 'waiting',
          sentBytes: 0,
        });
      }
    },
    [addFileCard, announceFile, startFileSend],
  );

  useEffect(() => {
    if (!id) {
      setStatusMessage('Error: No ID found in URL query string. Please append ?id=your_id to the URL.');
      return;
    }

    setStatusMessage(`Connecting shared clipboard with ID: ${id}`);
    wsManager.current?.close();

    const wsUrl = `/broad/sub?id=${encodeURIComponent(id)}`;
    const manager = new WebSocketManager(wsUrl, clipboardWsConfig);
    wsManager.current = manager;

    manager.onOpen(() => {
      setStatusMessage(`Connected. Shared clipboard ID: ${id}`);
      // 重连后旧连接的文件 offer 已失效，重新注册本地待传文件
      for (const [oldFileId, file] of [...localFiles.current.entries()]) {
        localFiles.current.delete(oldFileId);
        setFileCards((prev) => prev.filter((card) => card.fileId !== oldFileId));
        sendOfferOp(file, oldFileId);
      }
    });

    manager.onMessage(async (event) => {
      // text 帧为剪贴板内容，binary 帧为文件传输控制消息
      if (typeof event.data !== 'string') {
        try {
          let text: string | null = null;
          if (event.data instanceof Blob) {
            text = await event.data.text();
          } else if (event.data instanceof ArrayBuffer) {
            text = new TextDecoder().decode(event.data);
          }
          if (text) {
            handleControl(JSON.parse(text));
          }
        } catch (parseError) {
          console.error('Failed to parse control message:', parseError);
        }
        return;
      }

      const text = event.data;
      if (text === lastPublishedContent.current) {
        return;
      }

      isApplyingRemoteMessage.current = true;
      setContent(text);
      setLastSyncTime(formatSyncTime());
      window.setTimeout(() => {
        isApplyingRemoteMessage.current = false;
      }, 0);
    });

    manager.onClose((event) => {
      setStatusMessage(`Disconnected. ID: ${id}. Error Code: ${event.code}, Date: ${new Date()}`);
    });

    manager.onError((error) => {
      setStatusMessage(`WebSocket Error with ID: ${id}. See console for details.`);
      console.error(`Clipboard WebSocket Error with ID: ${id}:`, error);
    });

    manager.connect();

    return () => {
      if (publishTimer.current) {
        window.clearTimeout(publishTimer.current);
      }
      manager.close();
    };
  }, [id, handleControl, sendOfferOp]);

  const publishContent = useCallback(async (nextContent: string) => {
    if (!id) {
      return;
    }

    lastPublishedContent.current = nextContent;

    try {
      const response = await fetch(`/broad/pub?id=${encodeURIComponent(id)}`, {
        method: 'POST',
        headers: {
          'Content-Type': 'text/plain;charset=utf-8',
        },
        body: nextContent,
      });

      if (!response.ok) {
        throw new Error(`Publish failed with status ${response.status}`);
      }

      setLastSyncTime(formatSyncTime());
      setCopyStatus('');
    } catch (error) {
      console.error('Failed to publish clipboard content:', error);
      setStatusMessage('Error publishing clipboard content. See console for details.');
    }
  }, [id]);

  const schedulePublish = useCallback((nextContent: string) => {
    if (publishTimer.current) {
      window.clearTimeout(publishTimer.current);
    }

    publishTimer.current = window.setTimeout(() => {
      publishContent(nextContent);
    }, 300);
  }, [publishContent]);

  const handleContentChange = (nextContent: string) => {
    setContent(nextContent);

    if (!isApplyingRemoteMessage.current) {
      schedulePublish(nextContent);
    }
  };

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(content);
      setCopyStatus('Copied');
    } catch (error) {
      console.error('Failed to copy clipboard content:', error);
      setCopyStatus('Copy failed');
    }
  };

  const handleClear = () => {
    handleContentChange('');
  };

  const handleFilePick = (file: File) => {
    if (!id) {
      return;
    }
    sendOfferOp(file);
  };

  const handleDownload = async (card: FileCard) => {
    try {
      const response = await fetch(`/files/status/${encodeURIComponent(card.fileId)}`);
      const data = (await response.json()) as { available?: boolean };
      if (!data.available) {
        updateFileCard(card.fileId, { status: 'unavailable' });
        return;
      }
      updateFileCard(card.fileId, { status: 'download-started' });
      const anchor = document.createElement('a');
      anchor.href = `/files/download/${encodeURIComponent(card.fileId)}`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
    } catch (error) {
      console.error('Failed to download file:', error);
      updateFileCard(card.fileId, { status: 'error' });
    }
  };

  const openClipboard = () => {
    const trimmedId = clipboardId.trim();
    if (!trimmedId) {
      return;
    }

    window.location.href = `/?id=${encodeURIComponent(trimmedId)}`;
  };

  const statusIsError = statusMessage.toLowerCase().includes('error');

  const fileStatusText = (card: FileCard): string => {
    if (card.mine) {
      switch (card.status) {
        case 'sending':
          return card.size > 0 ? `Sending ${Math.floor((card.sentBytes / card.size) * 100)}%` : 'Sending';
        case 'sent':
          return 'Sent';
        case 'error':
          return 'Send failed';
        default:
          return 'Waiting for download';
      }
    }
    switch (card.status) {
      case 'download-started':
        return 'Download started';
      case 'unavailable':
        return 'Sender offline';
      case 'error':
        return 'Download failed';
      default:
        return 'Ready to download';
    }
  };

  return (
    <div className="min-h-screen bg-white">
      <main className="container mx-auto px-4 py-8">
        <div className="mb-6">
          <div>
            <a href="/handler" className="text-sm text-gray-600 underline hover:text-gray-900">
              Open custom message handler
            </a>
            <h1 className="mt-3 text-4xl font-bold text-gray-950">NOTIR Shared Clipboard</h1>
            <p className="mt-2 max-w-2xl text-sm text-gray-600">
              Paste or edit text here. Other online pages opened with the same ID will update automatically.
              Send a file and other pages can download it while you stay online.
            </p>
          </div>
        </div>

        {!id && (
          <section className="mb-6 rounded-lg border border-gray-200 bg-gray-50 p-4">
            <h2 className="text-lg font-semibold text-gray-900">Open a Clipboard</h2>
            <p className="mt-2 max-w-2xl text-sm text-gray-600">
              Choose an ID and open the same URL on another computer to share text while both pages are online.
            </p>
            <div className="mt-4 flex flex-col gap-2 sm:flex-row">
              <input
                type="text"
                value={clipboardId}
                onChange={(event) => setClipboardId(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    openClipboard();
                  }
                }}
                placeholder="clipboard-id"
                className="min-w-0 flex-1 rounded-md border border-gray-300 px-3 py-2 text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-200"
              />
              <button
                type="button"
                onClick={openClipboard}
                disabled={!clipboardId.trim()}
                className="rounded-md bg-blue-600 px-4 py-2 text-sm font-semibold text-white hover:bg-blue-700 disabled:bg-gray-300"
              >
                Open Clipboard
              </button>
            </div>
          </section>
        )}

        {statusIsError ? (
          <div className="mb-4 rounded-md border border-red-300 bg-red-50 p-3 text-sm text-red-700">
            {statusMessage}
          </div>
        ) : (
          <div className="mb-3 text-xs text-gray-400">{statusMessage}</div>
        )}

        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <div className="text-xs text-gray-400">
            <span>Synced</span>
            <span className="ml-2 font-mono text-gray-500">{lastSyncTime || '--:--:--'}</span>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {copyStatus && <span className="text-sm text-gray-600">{copyStatus}</span>}
            <button
              type="button"
              onClick={() => fileInput.current?.click()}
              disabled={!id}
              className="rounded-md bg-blue-600 px-4 py-2 text-sm font-semibold text-white hover:bg-blue-700 disabled:bg-gray-300"
            >
              Send File
            </button>
            <input
              ref={fileInput}
              type="file"
              className="hidden"
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) {
                  handleFilePick(file);
                }
                event.target.value = '';
              }}
            />
            <button
              type="button"
              onClick={handleCopy}
              disabled={!content}
              className="rounded-md bg-blue-600 px-4 py-2 text-sm font-semibold text-white hover:bg-blue-700 disabled:bg-gray-300"
            >
              Copy
            </button>
            <button
              type="button"
              onClick={handleClear}
              disabled={!content}
              className="rounded-md bg-gray-600 px-4 py-2 text-sm font-semibold text-white hover:bg-gray-700 disabled:bg-gray-300"
            >
              Clear
            </button>
          </div>
        </div>

        {fileCards.length > 0 && (
          <section className="mb-4 space-y-2" aria-label="Shared files">
            {fileCards.map((card) => (
              <div
                key={card.fileId}
                className="rounded-md border border-gray-200 bg-gray-50 px-4 py-3"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium text-gray-900">{card.name}</div>
                    <div className="text-xs text-gray-500">
                      {formatBytes(card.size)}
                      <span className="mx-1">·</span>
                      <span className={card.status === 'unavailable' || card.status === 'error' ? 'text-red-600' : ''}>
                        {fileStatusText(card)}
                      </span>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {!card.mine && (card.status === 'waiting' || card.status === 'error') && (
                      <button
                        type="button"
                        onClick={() => handleDownload(card)}
                        className="rounded-md bg-blue-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-blue-700"
                      >
                        Download
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => removeFileCard(card.fileId)}
                      aria-label={`Remove ${card.name}`}
                      className="rounded-md px-2 py-1 text-sm text-gray-400 hover:bg-gray-200 hover:text-gray-700"
                    >
                      ×
                    </button>
                  </div>
                </div>
                {card.mine && card.status === 'sending' && (
                  <div className="mt-2 h-1 w-full overflow-hidden rounded bg-gray-200">
                    <div
                      className="h-1 rounded bg-blue-600 transition-all"
                      style={{ width: `${card.size > 0 ? Math.floor((card.sentBytes / card.size) * 100) : 100}%` }}
                    />
                  </div>
                )}
              </div>
            ))}
          </section>
        )}

        <textarea
          value={content}
          onChange={(event) => handleContentChange(event.target.value)}
          disabled={!id}
          spellCheck={false}
          placeholder="Paste text here..."
          className="min-h-[55vh] w-full resize-y rounded-md border border-gray-300 p-4 font-mono text-sm leading-6 text-gray-950 shadow-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-200 disabled:bg-gray-100"
        />
      </main>
    </div>
  );
}

export default ClipboardPage;
