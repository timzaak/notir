import React from 'react';
import type { WebSocketConfig } from '../utils/WebSocketManager';

interface WebSocketConfigProps {
  config: WebSocketConfig;
  onConfigChange: (config: WebSocketConfig) => void;
}

const WebSocketConfigComponent: React.FC<WebSocketConfigProps> = ({
  config,
  onConfigChange,
}) => {
  const handleConfigChange = (field: keyof WebSocketConfig, value: boolean | number) => {
    onConfigChange({
      ...config,
      [field]: value
    });
  };

  return (
    <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 mb-5">
      <h3 className="text-lg font-semibold mb-3 text-gray-800">WebSocket Config</h3>
      
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">

        <div className="flex flex-col space-y-1 items-center">
          <label htmlFor="enableReconnect" className="text-sm font-medium text-gray-700">
            auto connect
          </label>
          <input
            type="checkbox"
            id="enableReconnect"
            checked={config.enableReconnect}
            onChange={(e) => handleConfigChange('enableReconnect', e.target.checked)}
            className="text-blue-600 bg-gray-100 border-gray-300 rounded focus:ring-blue-500 focus:ring-2 disabled:opacity-50"
          />

        </div>

        <div className="flex flex-col space-y-1">
          <label htmlFor="reconnectInterval" className="text-sm font-medium text-gray-700">
            Interval(ms)
          </label>
          <input
            type="number"
            id="reconnectInterval"
            min="1000"
            max="30000"
            step="1000"
            value={config.reconnectInterval}
            onChange={(e) => handleConfigChange('reconnectInterval', parseInt(e.target.value) || 3000)}
            className="px-3 py-1 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-50 disabled:bg-gray-100"
          />
        </div>

        <div className="flex flex-col space-y-1">
          <label htmlFor="maxReconnectAttempts" className="text-sm font-medium text-gray-700">
            Max Retries
          </label>
          <input
            type="number"
            id="maxReconnectAttempts"
            min="1"
            max="20"
            value={config.maxReconnectAttempts}
            onChange={(e) => handleConfigChange('maxReconnectAttempts', parseInt(e.target.value) || 5)}
            className="px-3 py-1 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-50 disabled:bg-gray-100"
          />
        </div>
      </div>
    </div>
  );
};

export default WebSocketConfigComponent;