const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('mewQuery', ({ request, onSuccess, onFailure }) => {
  ipcRenderer.invoke('mew-composition:query', request).then(
    (response) => onSuccess?.(JSON.stringify(response)),
    (error) => onFailure?.(1, error.message),
  );
});
