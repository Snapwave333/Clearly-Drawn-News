const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('atlasOffice', {
  getBootstrap: () => ipcRenderer.invoke('office:bootstrap'),
  getMessages: () => ipcRenderer.invoke('office:messages'),
  loadState: () => ipcRenderer.invoke('office:state:load'),
  saveState: state => ipcRenderer.invoke('office:state:save', state),
  showWindow: () => ipcRenderer.invoke('office:window:show'),
  launchUnreal: () => ipcRenderer.invoke('office:launch-unreal')
})
