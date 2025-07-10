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
const defaultEditorCode = `// event: MessageEvent, arrayBufferToBase64: (buffer: ArrayBuffer) => string
(event, arrayBufferToBase64) => {
  if (typeof event.data === 'string') {
    console.log(event.data);
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
  const [statusMessage, setStatusMessage] = useState('<em>Checking for ID in URL...</em>');
  const ws = useRef<WebSocket | null>(null);
  const heartbeatIntervalId = useRef<number | null>(null);
  const [editorCode, setEditorCode] = useState(() => {
    return localStorage.getItem('wsMessageHandlerCode') || defaultEditorCode;
  });
  // Explicitly type wsMessageHandler state
  const [wsMessageHandler, setWsMessageHandler] = useState<(event: MessageEvent) => void>(() => defaultWsMessageHandler);
  const [isApplyingCode, setIsApplyingCode] = useState(false);

  // Function to compile and apply the WebSocket message handler code
  const compileAndApplyCode = useCallback(async (codeToApply: string, isInitialLoad: boolean = false) => {
    if (!isInitialLoad) {
      setIsApplyingCode(true);
      setStatusMessage("<em>Applying new code...</em>");
      // Short delay to allow UI to update before potentially blocking compilation
      await new Promise(resolve => setTimeout(resolve, 50));
    } else {
      setStatusMessage("<em>Initializing WebSocket handler...</em>");
    }

    try {
      const dynamicHandler = new Function('event', 'arrayBufferToBase64', codeToApply.startsWith('(') ? `return ${codeToApply}(event, arrayBufferToBase64)` : codeToApply);

      setWsMessageHandler(() => (event: MessageEvent) => {
        try {
          dynamicHandler(event, arrayBufferToBase64, (msg: string) => setStatusMessage(`<em>${msg}</em>`));
        } catch (e) {
          console.error(`Error executing dynamic WebSocket message handler (loaded from ${isInitialLoad ? 'storage/default' : 'editor'}):`, e);
          setStatusMessage(`<em>Error in custom message handler. Check console. Using default handler.</em>`);
          defaultWsMessageHandler(event);
        }
      });

      if (!isInitialLoad) {
        localStorage.setItem('wsMessageHandlerCode', codeToApply);
        setStatusMessage("<em>WebSocket message handler updated successfully!</em>");
      }
    } catch (error) {
      console.error(`Error compiling WebSocket message handler (loaded from ${isInitialLoad ? 'storage/default' : 'editor'}):`, error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      setStatusMessage(`<em>Error compiling code: ${errorMessage}. ${isInitialLoad ? 'Using default handler.' : 'Previous handler remains active.'}</em>`);
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

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    let id = params.get('id');
    if(import.meta.env.DEV && !id) {
      id = 'test'
    }

    if (!id) {
      setStatusMessage('<em>Error: No ID found in URL query string. Please append ?id=your_id to the URL.</em>');
      return;
    }

    setStatusMessage(`<em>Attempting to connect WebSocket with ID: ${id}</em>`);

    const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = import.meta.env.DEV ? `ws://127.0.0.1:5800/sub?id=${id}`: `${wsProtocol}//${window.location.host}/sub?id=${id}`;

    ws.current = new WebSocket(wsUrl);

    ws.current.onopen = () => {
      setStatusMessage(`<em>Connected with ID: ${id}!</em>`);
      console.log(`WebSocket connected with ID: ${id}`);

      heartbeatIntervalId.current = window.setInterval(() => {
        if (ws.current && ws.current.readyState === WebSocket.OPEN) {
          ws.current.send('!');
        }
      }, 30000);
    };

    ws.current.onmessage = (event) => {
      // Use the current wsMessageHandler state
      wsMessageHandler(event);
    };

    ws.current.onclose = (event) => {
      setStatusMessage(`<em>Disconnected. ID: ${id}. Code: ${event.code}, Reason: ${event.reason || 'N/A'}</em>`);
      if (heartbeatIntervalId.current) {
        clearInterval(heartbeatIntervalId.current);
        heartbeatIntervalId.current = null;
      }
    };

    ws.current.onerror = (error) => {
      setStatusMessage(`<em>WebSocket Error with ID: ${id}. See console for details.</em>`);
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

  return (
    <div className="container mx-auto px-4 py-8 text-center">
      <h1 className="text-5xl font-bold mb-5">NOTIR</h1>
      <div id="status" dangerouslySetInnerHTML={{ __html: statusMessage }} />
      <CodeEditor
        code={editorCode}
        setCode={setEditorCode}
        submitCode={handleCodeSubmit}
        isLoading={isApplyingCode}
      />
      <div id="devtools-shortcut" className="mt-4">
        <p className="text-sm text-gray-600">
          Press Ctrl+Shift+J (Windows/Linux) or Cmd+Option+J (Mac) to open the Developer Console to see messages.
        </p>
      </div>
    </div>
  );
}

export default App;
