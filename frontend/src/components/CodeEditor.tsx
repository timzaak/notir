import React from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { javascript } from '@codemirror/lang-javascript';
// import { githubLight } from '@uiw/codemirror-theme-github';

interface CodeEditorProps {
  code: string;
  setCode: (code: string) => void;
  submitCode: () => void;
  isLoading?: boolean;
}

const CodeEditor: React.FC<CodeEditorProps> = ({ code, setCode, submitCode, isLoading }) => {
  const handleCodeChange = (value: string) => {
    setCode(value);
  };

  const handleSubmit = () => {
    submitCode();
  };

  return (
    <div className="code-editor-container mt-6 w-full max-w-2xl mx-auto">
      <h2 className="text-2xl font-semibold mb-3 text-left">WebSocket Message Handler Code</h2>
      <CodeMirror
        value={code}
        height="320px"
        extensions={[javascript({ jsx: true })]}
        onChange={handleCodeChange}
        readOnly={isLoading}
        className="border border-gray-300 rounded-md shadow-sm text-sm text-left"
      />
      <button
        onClick={handleSubmit}
        disabled={isLoading}
        className="mt-4 px-6 py-2 bg-green-500 text-white font-semibold rounded-md hover:bg-green-600 focus:outline-none focus:ring-2 focus:ring-green-500 focus:ring-opacity-50 disabled:bg-gray-400" // Tailwind classes for button styling
      >
        {isLoading ? 'Applying...' : 'Apply and Save Code'}
      </button>
      <p className="mt-2 text-xs text-gray-500 text-left">
        Edit the JavaScript function body above to customize how WebSocket messages are handled, it now outputs receiving data to dev console.
        The function will receive: <code>event</code> (MessageEvent), <code>arrayBufferToBase64</code> (function).
      </p>
    </div>
  );
};

export default CodeEditor;
