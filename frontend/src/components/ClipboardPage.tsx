import { useCallback, useEffect, useRef, useState } from 'react';
import { WebSocketManager, type WebSocketConfig } from '../utils/WebSocketManager';

const clipboardWsConfig: WebSocketConfig = {
  enableReconnect: true,
  reconnectInterval: 3000,
  maxReconnectAttempts: 20,
  mode: 'broad',
};

const readMessageText = async (data: MessageEvent['data']): Promise<string | null> => {
  if (typeof data === 'string') {
    return data;
  }

  if (data instanceof Blob) {
    return data.text();
  }

  return null;
};

const formatSyncTime = () => new Date().toLocaleTimeString();

function ClipboardPage() {
  const params = new URLSearchParams(window.location.search);
  const id = params.get('id')?.trim() || '';

  const [clipboardId, setClipboardId] = useState(id);
  const [content, setContent] = useState('');
  const [statusMessage, setStatusMessage] = useState('Checking for ID in URL...');
  const [lastSyncTime, setLastSyncTime] = useState('');
  const [copyStatus, setCopyStatus] = useState('');

  const wsManager = useRef<WebSocketManager | null>(null);
  const publishTimer = useRef<number | null>(null);
  const isApplyingRemoteMessage = useRef(false);
  const lastPublishedContent = useRef<string | null>(null);

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
    });

    manager.onMessage(async (event) => {
      const text = await readMessageText(event.data);
      if (text === null || text === lastPublishedContent.current) {
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
  }, [id]);

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

  const openClipboard = () => {
    const trimmedId = clipboardId.trim();
    if (!trimmedId) {
      return;
    }

    window.location.href = `/?id=${encodeURIComponent(trimmedId)}`;
  };

  const statusIsError = statusMessage.toLowerCase().includes('error');

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
