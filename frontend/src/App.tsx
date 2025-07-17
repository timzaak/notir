import { useEffect, useState, useRef, useCallback } from 'react';
import CodeEditor from './components/CodeEditor';

// 工具函数
const arrayBufferToBase64 = (buffer: ArrayBuffer): string => {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return window.btoa(binary);
};

// 默认消息处理器
const defaultWsMessageHandler = (event: MessageEvent) => {
  const { data } = event;

  if (typeof data === 'string') {
    console.log(data);
  } else if (data instanceof ArrayBuffer) {
    console.log(arrayBufferToBase64(data));
  } else if (data instanceof Blob) {
    const reader = new FileReader();
    reader.onload = () => {
      console.log(arrayBufferToBase64(reader.result as ArrayBuffer));
    };
    reader.readAsArrayBuffer(data);
  } else {
    console.warn("Received unknown message type:", data);
  }
};

// 默认编辑器代码
const defaultEditorCode = `// event: MessageEvent, arrayBufferToBase64: (buffer: ArrayBuffer) => string, sendMessage: (message: string | ArrayBufferLike | Blob | ArrayBufferView) => void
(event, arrayBufferToBase64, sendMessage) => {
  const { data } = event;
  
  if (typeof data === 'string') {
    console.log(data);
    // Attention: sendMessage only works when publish message via /pub?id=\${userId}&mod=ping_pong
    // sendMessage('response');
  } else if (data instanceof ArrayBuffer) {
    console.log(arrayBufferToBase64(data));
  } else if (data instanceof Blob) {
    const reader = new FileReader();
    reader.onload = () => {
      console.log(arrayBufferToBase64(reader.result));
    };
    reader.readAsArrayBuffer(data);
  } else {
    console.warn("Received unknown message type:", data);
  }
}
`;

function App() {
  const [statusMessage, setStatusMessage] = useState('Checking for ID in URL...');
  const [editorCode, setEditorCode] = useState(() =>
    localStorage.getItem('wsMessageHandlerCode') || defaultEditorCode
  );
  const [wsMessageHandler, setWsMessageHandler] = useState<(event: MessageEvent) => void>(() => defaultWsMessageHandler);
  const [isApplyingCode, setIsApplyingCode] = useState(false);
  const [versionInfo, setVersionInfo] = useState('');

  const ws = useRef<WebSocket | null>(null);
  const heartbeatIntervalId = useRef<number | null>(null);
  const wsMessageHandlerRef = useRef(wsMessageHandler);

  useEffect(() => {
    wsMessageHandlerRef.current = wsMessageHandler;
  }, [wsMessageHandler]);

  // 编译并应用 WebSocket 消息处理器代码
  const compileAndApplyCode = useCallback(async (codeToApply: string, isInitialLoad = false) => {
    const statusMsg = isInitialLoad ? "Initializing WebSocket handler..." : "Applying new code...";
    setStatusMessage(statusMsg);

    if (!isInitialLoad) {
      setIsApplyingCode(true);
      await new Promise(resolve => setTimeout(resolve, 50));
    }

    try {
      const dynamicHandler = new Function('event', 'arrayBufferToBase64', 'sendMessage',
        `(${codeToApply})(event, arrayBufferToBase64, sendMessage)`
      );

      const createSendMessage = () => (message: string | ArrayBufferLike | Blob | ArrayBufferView) => {
        if (ws.current?.readyState === WebSocket.OPEN) {
          ws.current.send(message);
        } else {
          console.error("WebSocket is not connected.");
        }
      };

      setWsMessageHandler(() => (event: MessageEvent) => {
        try {
          dynamicHandler(event, arrayBufferToBase64, createSendMessage());
        } catch (e) {
          console.error(`Error executing WebSocket handler:`, e);
          setStatusMessage(`Error in custom message handler. Using default handler.`);
          defaultWsMessageHandler(event);
        }
      });

      if (!isInitialLoad) {
        localStorage.setItem('wsMessageHandlerCode', codeToApply);
        setStatusMessage("WebSocket message handler updated successfully!");
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      console.error(`Error compiling WebSocket handler:`, error);
      setStatusMessage(`Error compiling code: ${errorMessage}. ${isInitialLoad ? 'Using default handler.' : 'Previous handler remains active.'}`);

      if (isInitialLoad) {
        setWsMessageHandler(() => defaultWsMessageHandler);
      }
    } finally {
      if (!isInitialLoad) {
        setIsApplyingCode(false);
      }
    }
  }, []);

  // Effect for applying initial code on mount
  useEffect(() => {
    const initialCode = localStorage.getItem('wsMessageHandlerCode') || defaultEditorCode;
    setEditorCode(initialCode);
    compileAndApplyCode(initialCode, true);
  }, [compileAndApplyCode]); // compileAndApplyCode is stable due to useCallback

  const handleCodeSubmit = () => {
    compileAndApplyCode(editorCode, false);
  }

  const resetCode = () => {
    setEditorCode(defaultEditorCode);
  }

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const id = params.get('id') || (import.meta.env.DEV ? 'test' : null);

    if (!id) {
      setStatusMessage('Error: No ID found in URL query string. Please append ?id=your_id to the URL.');
      return;
    }

    setStatusMessage(`Attempting to connect WebSocket with ID: ${id}`);
    ws.current = new WebSocket(`/sub?id=${id}`);

    const clearHeartbeat = () => {
      if (heartbeatIntervalId.current) {
        clearInterval(heartbeatIntervalId.current);
        heartbeatIntervalId.current = null;
      }
    };

    ws.current.onopen = () => {
      setStatusMessage(`Connected with ID: ${id}`);
      console.log(`WebSocket connected with ID: ${id}`);

      heartbeatIntervalId.current = window.setInterval(() => {
        if (ws.current?.readyState === WebSocket.OPEN) {
          ws.current.send('!');
        }
      }, 30000);
    };

    ws.current.onmessage = (event) => wsMessageHandlerRef.current(event);

    ws.current.onclose = (event) => {
      setStatusMessage(`Disconnected. ID: ${id}. Error Code: ${event.code}, Reason: ${event.reason || 'N/A'}`);
      clearHeartbeat();
    };

    ws.current.onerror = (error) => {
      setStatusMessage(`WebSocket Error with ID: ${id}. See console for details.`);
      console.error(`WebSocket Error with ID: ${id}:`, error);
      clearHeartbeat();
    };

    return () => {
      ws.current?.close();
      clearHeartbeat();
    };
  }, []);

  useEffect(() => {
    // const httpPrefix = import.meta.env.DEV ? `http://localhost:5800/`: `/`;
    fetch(`/version`)
      .then(response => response.text())
      .then(data => setVersionInfo(data))
      .catch(error => console.error('Error fetching version:', error));
  }, []);

  const statusIsError = statusMessage.toLowerCase().includes('error');
  const statusMessageClasses = `
    p-3 mb-5 rounded-md shadow-md text-left
    ${statusIsError
      ? 'bg-red-100 text-red-700 border border-red-300'
      : 'bg-blue-100 text-blue-800 border border-blue-300'
    }
  `;

  return (
    <div className="container mx-auto px-4 py-8 text-center">
      <h1 className="text-5xl font-bold mb-5">NOTIR</h1>
      <div id="status" className={statusMessageClasses.trim()}>{statusMessage}</div>
      <div id="devtools-shortcut" className="mt-4 text-left">
        <p className="text-sm text-gray-600">
          Press Ctrl+Shift+J (Windows/Linux) or Cmd+Option+J (Mac) to see message.
        </p>
      </div>
      <CodeEditor
        code={editorCode}
        setCode={setEditorCode}
        submitCode={handleCodeSubmit}
        resetCode={resetCode}
        isLoading={isApplyingCode}
      />
      <footer className="mt-8 text-sm text-gray-500">
        <p className='text-gray-800'>If you have any issues, please report them on <a href="https://github.com/timzaak/notir/issues?utm_source=notir" target="_blank" rel="noopener noreferrer" className="underline hover:text-gray-700">GitHub Issue</a>.</p>
        <br />
        <p>
          <a href="https://github.com/timzaak/notir?utm_source=notir" target="_blank" rel="noopener noreferrer" className="underline hover:text-gray-700">Source Code</a>
          {' | '}
          <a href="https://blog.fornetcode.com?utm_source=notir" target="_blank" rel="noopener noreferrer" className="underline hover:text-gray-700">Blog</a>
          {versionInfo && (
            <>
              {' | '}
              <a href="https://github.com/timzaak/notir/releases?utm_source=notir" target="_blank" rel="noopener noreferrer" className="underline hover:text-gray-700">
                v{versionInfo}
              </a>
            </>
          )}
        </p>
      </footer>
    </div>
  );
}

export default App;
