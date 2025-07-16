import { useEffect, useState, useRef, useCallback } from 'react';
import CodeEditor from './components/CodeEditor';


const defaultWsMessageHandler = (event: MessageEvent) => {
  if (typeof event.data === 'string') {
    console.log(event.data);
  } else if (event.data instanceof ArrayBuffer) {
    const base64String = arrayBufferToBase64(event.data);
    console.log(base64String);
  } else if (event.data instanceof Blob) {
    // console.log("Received binary message (Blob)");
    const reader = new FileReader();
    reader.onload = function() {
      const base64String = arrayBufferToBase64(reader.result as ArrayBuffer);
      console.log(base64String);
    };
    reader.readAsArrayBuffer(event.data);
  } else {
    console.warn("Received unknown message type:", event.data);
  }
};

// Helper function (can be outside the component or imported)
function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = '';
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return window.btoa(binary);
}

// Default code for the editor
const defaultEditorCode = `// event: MessageEvent, arrayBufferToBase64: (buffer: ArrayBuffer) => string, sendMessage: (message: string | ArrayBufferLike | Blob | ArrayBufferView) => void
(event, arrayBufferToBase64, sendMessage) => {
  if (typeof event.data === 'string') {
    console.log(event.data);
    // Attention: sendMessage only works when publish message via /pub?id=\${userId}&mod=ping_pong
    // sendMessage('response');
  } else if (event.data instanceof ArrayBuffer) {
    const base64String = arrayBufferToBase64(event.data);
    console.log(base64String);
  } else if (event.data instanceof Blob) {
    const reader = new FileReader();
    reader.onload = function() {
      const base64String = arrayBufferToBase64(reader.result);
      console.log(base64String);
    };
    reader.readAsArrayBuffer(event.data);
  } else {
    console.warn("Received unknown message type:", event.data);
  }
}
`;

function App() {
  const [statusMessage, setStatusMessage] = useState('Checking for ID in URL...');
  const ws = useRef<WebSocket | null>(null);
  const heartbeatIntervalId = useRef<number | null>(null);
  const [editorCode, setEditorCode] = useState(() => {
    return localStorage.getItem('wsMessageHandlerCode') || defaultEditorCode;
  });
  // Explicitly type wsMessageHandler state
  const [wsMessageHandler, setWsMessageHandler] = useState<(event: MessageEvent) => void>(() => defaultWsMessageHandler);
  const [isApplyingCode, setIsApplyingCode] = useState(false);
  const [versionInfo, setVersionInfo] = useState('');

  const wsMessageHandlerRef = useRef(wsMessageHandler);
  useEffect(() => {
    wsMessageHandlerRef.current = wsMessageHandler
  }, [wsMessageHandler])

  // Function to compile and apply the WebSocket message handler code
  const compileAndApplyCode = useCallback(async (codeToApply: string, isInitialLoad: boolean = false) => {
    if (!isInitialLoad) {
      setIsApplyingCode(true);
      setStatusMessage("Applying new code...");
      // Short delay to allow UI to update before potentially blocking compilation
      await new Promise(resolve => setTimeout(resolve, 50));
    } else {
      setStatusMessage("Initializing WebSocket handler...");
    }
    try {
      const dynamicHandler = new Function('event', 'arrayBufferToBase64', 'sendMessage', `(${codeToApply})(event, arrayBufferToBase64, sendMessage)` )
      setWsMessageHandler(() => (event: MessageEvent) => {
        try {
          const sendMessage = (message: string | ArrayBufferLike | Blob | ArrayBufferView) => {
            if (ws.current && ws.current.readyState === WebSocket.OPEN) {
              ws.current.send(message);
            } else {
              console.error("WebSocket is not connected.");
            }
          };
          dynamicHandler(event, arrayBufferToBase64, sendMessage);
        } catch (e) {
          console.error(`Error executing dynamic WebSocket message handler (loaded from ${isInitialLoad ? 'storage/default' : 'editor'}):`, e);
          setStatusMessage(`Error in custom message handler. Check console. Using default handler.`);
          defaultWsMessageHandler(event);
        }
      });

      if (!isInitialLoad) {
        localStorage.setItem('wsMessageHandlerCode', codeToApply);
        setStatusMessage("WebSocket message handler updated successfully!");
      }
    } catch (error) {
      console.error(`Error compiling WebSocket message handler (loaded from ${isInitialLoad ? 'storage/default' : 'editor'}):`, error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      setStatusMessage(`Error compiling code: ${errorMessage}. ${isInitialLoad ? 'Using default handler.' : 'Previous handler remains active.'}`);
      if (isInitialLoad) {
        // Fallback to default handler if initial code from storage is bad
        setWsMessageHandler(() => defaultWsMessageHandler);
      }
    } finally {
      if (!isInitialLoad) {
        setIsApplyingCode(false);
      }
    }
  }, [setStatusMessage]); // editorCode is not a dependency here, it's passed as an argument

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
    let id = params.get('id');
    if(import.meta.env.DEV && !id) {
      id = 'test'
    }

    if (!id) {
      setStatusMessage('Error: No ID found in URL query string. Please append ?id=your_id to the URL.');
      return;
    }

    setStatusMessage(`Attempting to connect WebSocket with ID: ${id}`);

    // const wsUrl = `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}/sub?id=${id}`;
    ws.current = new WebSocket(`/sub?id=${id}`);

    ws.current.onopen = () => {
      setStatusMessage(`Connected with ID: ${id}`);
      console.log(`WebSocket connected with ID: ${id}`);

      heartbeatIntervalId.current = window.setInterval(() => {
        if (ws.current && ws.current.readyState === WebSocket.OPEN) {
          ws.current.send('!');
        }
      }, 30000);
    };

    ws.current.onmessage = (event) => {
      wsMessageHandlerRef.current(event);
    };

    ws.current.onclose = (event) => {
      setStatusMessage(`Disconnected. ID: ${id}. Error Code: ${event.code}, Reason: ${event.reason || 'N/A'}`);
      if (heartbeatIntervalId.current) {
        clearInterval(heartbeatIntervalId.current);
        heartbeatIntervalId.current = null;
      }
    };

    ws.current.onerror = (error) => {
      setStatusMessage(`WebSocket Error with ID: ${id}. See console for details.`);
      console.error(`WebSocket Error with ID: ${id}:`, error);
      if (heartbeatIntervalId.current) {
        clearInterval(heartbeatIntervalId.current);
        heartbeatIntervalId.current = null;
      }
    };

    // Cleanup function
    return () => {
      if (ws.current) {
        ws.current.close();
      }
      if (heartbeatIntervalId.current) {
        clearInterval(heartbeatIntervalId.current);
      }
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
        <br/>
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
