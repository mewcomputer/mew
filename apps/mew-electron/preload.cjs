const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('mewQuery', ({ request, onSuccess, onFailure }) => {
  ipcRenderer.invoke('mew:query', request).then(
    (response) => onSuccess?.(JSON.stringify(response)),
    (error) => onFailure?.(1, error.message),
  );
});

contextBridge.exposeInMainWorld('mewListen', (listener) => {
  const receive = (_event, payload) => listener(payload);
  ipcRenderer.on('mew:event', receive);
  return () => ipcRenderer.removeListener('mew:event', receive);
});
