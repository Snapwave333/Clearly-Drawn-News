const { app, BrowserWindow, Menu, Tray, ipcMain, nativeImage } = require('electron')
const fs = require('node:fs')
const path = require('node:path')
const os = require('node:os')

let mainWindow
let tray
let quitting = false

const projectRoot = path.resolve(__dirname, '..', '..')
const registryPath = path.join(projectRoot, '.opencode', 'config', 'agents.json')
const iconPath = path.join(__dirname, '..', 'icon.ico')

const logFile = path.join(__dirname, '..', 'main.log')
function log(...args) {
  try {
    fs.appendFileSync(logFile, `[${new Date().toISOString()}] ${args.join(' ')}\n`)
  } catch {}
}

log('Starting electron main process, pid:', process.pid, 'argv:', process.argv.join(' '))
process.on('uncaughtException', err => log('UNCAUGHT EXCEPTION:', err.stack || err))
process.on('unhandledRejection', err => log('UNHANDLED REJECTION:', err.stack || err))

// Single instance lock (safe without exiting)
try {
  const gotTheLock = app.requestSingleInstanceLock()
  log('gotTheLock:', gotTheLock)
  if (gotTheLock) {
    app.on('second-instance', () => {
      log('second-instance triggered, showing window')
      showWindow()
    })
  }
} catch (e) {
  log('Single instance check exception:', e)
}

function dataCandidates() {
  const local = process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
  const roaming = process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming')
  const configured = process.env.ATLAS_DATA_DIR
  return [
    configured && path.join(configured, 'atlas.db'),
    path.join(local, 'atlas-studio', 'atlas', 'data', 'atlas.db'),
    path.join(roaming, 'atlas-studio', 'atlas', 'data', 'atlas.db'),
    path.join(projectRoot, 'artifacts', 'policy-smoke-v3', 'atlas.db'),
    path.join(projectRoot, 'artifacts', 'policy-smoke-v2', 'atlas.db'),
    path.join(projectRoot, 'artifacts', 'policy-smoke', 'atlas.db'),
    path.join(projectRoot, 'data', 'atlas.db')
  ].filter(Boolean)
}

function findDatabase() {
  return dataCandidates().find(candidate => fs.existsSync(candidate)) || null
}

function readMessages() {
  const databasePath = findDatabase()
  if (!databasePath) return { databasePath: null, messages: [] }
  try {
    const { DatabaseSync } = require('node:sqlite')
    const database = new DatabaseSync(databasePath, { readOnly: true })
    const messages = database.prepare(`
      SELECT message_id, project_id, from_agent, to_agent, message_type,
             priority, feedback_json, created_at
      FROM messages
      ORDER BY created_at DESC
      LIMIT 100
    `).all().map(row => ({
      ...row,
      feedback: JSON.parse(row.feedback_json || '[]')
    }))
    database.close()
    return { databasePath, messages }
  } catch (error) {
    return { databasePath, messages: [], error: error.message }
  }
}

function createTrayImage() {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#173f4f"/><path d="M7 23 15.8 6 25 23h-5l-1.8-4H13l-1.8 4Zm7.5-8h2.2l-1.1-2.8Z" fill="#f2a65a"/></svg>`
  return nativeImage.createFromDataURL(`data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`).resize({ width: 16, height: 16 })
}

function showWindow() {
  log('showWindow called, mainWindow exists:', !!mainWindow)
  if (!mainWindow) {
    createWindow()
  } else {
    if (mainWindow.isMinimized()) mainWindow.restore()
    mainWindow.show()
    mainWindow.focus()
  }
}

function createWindow() {
  log('createWindow executing')
  mainWindow = new BrowserWindow({
    width: 1720,
    height: 1060,
    minWidth: 1120,
    minHeight: 720,
    backgroundColor: '#e8e2d6',
    autoHideMenuBar: true,
    show: true, // Visible immediately
    icon: fs.existsSync(iconPath) ? iconPath : undefined,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  mainWindow.webContents.on('did-fail-load', (e, code, desc, url) => {
    log('did-fail-load:', code, desc, url)
  })

  mainWindow.webContents.on('console-message', (e, level, msg, line, src) => {
    log(`[renderer console ${level}] ${msg} (${path.basename(src || '')}:${line})`)
  })

  mainWindow.webContents.on('render-process-gone', (e, details) => {
    log('render-process-gone:', details.reason)
  })

  const devUrl = process.env.ATLAS_OFFICE_DEV_URL
  if (devUrl) {
    log('Loading dev URL:', devUrl)
    mainWindow.loadURL(devUrl)
  } else {
    const htmlPath = path.join(__dirname, '..', 'dist', 'index.html')
    log('Loading HTML file:', htmlPath, 'exists:', fs.existsSync(htmlPath))
    mainWindow.loadFile(htmlPath)
  }

  mainWindow.show()

  // Minimize to tray on close
  mainWindow.on('close', event => {
    log('mainWindow close event, quitting:', quitting)
    if (!quitting) {
      event.preventDefault()
      mainWindow.hide()
      log('mainWindow hidden to system tray')
    }
  })

  mainWindow.on('closed', () => {
    log('mainWindow closed event')
    mainWindow = null
  })
}

function updateTrayMenu() {
  if (!tray) return
  const denverTimeStr = new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/Denver',
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
    hour12: true
  }).format(new Date())

  tray.setToolTip(`Clearly Drawn Studios HQ — 38 Agents (Denver: ${denverTimeStr})`)
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: 'Open Clearly Drawn Studios HQ (3D)', click: showWindow },
    {
      label: '⚡ Launch Unreal Virtual Spectator View (3D Office)',
      click: () => {
        const { exec } = require('node:child_process')
        exec('cmd.exe /c "C:\\Users\\chrom\\Desktop\\Marker\\AtlasStudioOffice\\Launch_Atlas_Studio_Unreal.bat"')
      }
    },
    { type: 'separator' },
    { label: `Denver Time: ${denverTimeStr}`, enabled: false },
    { label: 'Staffing: 38 Autonomous Agents', enabled: false },
    { type: 'separator' },
    {
      label: 'Quit simulation completely',
      click: () => {
        quitting = true
        app.quit()
      }
    }
  ]))
}

function createTray() {
  log('createTray executing')
  tray = new Tray(createTrayImage())
  updateTrayMenu()
  tray.on('double-click', showWindow)
  tray.on('click', showWindow)
  setInterval(updateTrayMenu, 15000)
  log('createTray completed')
}

app.whenReady().then(() => {
  log('app.whenReady event fired')
  createWindow()
  createTray()
  app.on('activate', showWindow)
}).catch(err => {
  log('app.whenReady error:', err.stack || err)
})

app.on('window-all-closed', () => {
  log('window-all-closed fired')
})

app.on('before-quit', () => {
  log('before-quit fired')
  quitting = true
})

ipcMain.handle('office:bootstrap', () => {
  const registry = JSON.parse(fs.readFileSync(registryPath, 'utf8'))
  const feed = readMessages()
  return { agents: registry.roles, ...feed }
})

ipcMain.handle('office:messages', () => readMessages())

ipcMain.handle('office:state:load', () => {
  const statePath = path.join(app.getPath('userData'), 'office-state.json')
  if (!fs.existsSync(statePath)) return null
  try {
    return JSON.parse(fs.readFileSync(statePath, 'utf8'))
  } catch {
    return null
  }
})

ipcMain.handle('office:state:save', (_, state) => {
  const statePath = path.join(app.getPath('userData'), 'office-state.json')
  fs.mkdirSync(path.dirname(statePath), { recursive: true })
  fs.writeFileSync(statePath, JSON.stringify(state))
  return true
})

ipcMain.handle('office:window:show', () => {
  showWindow()
  return true
})

ipcMain.handle('office:launch-unreal', async () => {
  const { spawn } = require('node:child_process')
  const batPath = 'C:\\Users\\chrom\\Desktop\\Marker\\AtlasStudioOffice\\Launch_Atlas_Studio_Unreal.bat'
  const uprojectPath = 'C:\\Users\\chrom\\Desktop\\Marker\\AtlasStudioOffice\\AtlasStudioOffice.uproject'
  const ueExe = 'C:\\Program Files\\Epic Games\\UE_5.8\\Engine\\Binaries\\Win64\\UnrealEditor.exe'

  log('office:launch-unreal IPC invoked')
  try {
    if (fs.existsSync(batPath)) {
      log('Executing spectator batPath:', batPath)
      const p = spawn('cmd.exe', ['/c', batPath], { detached: true, stdio: 'ignore' })
      p.unref()
      return { success: true }
    } else if (fs.existsSync(ueExe) && fs.existsSync(uprojectPath)) {
      const args = [uprojectPath, '-game', '-dx12', '-sm6', '-windowed', '-ResX=1920', '-ResY=1080', '-WinX=60', '-WinY=30', '-messaging']
      const p = spawn(ueExe, args, { detached: true, stdio: 'ignore' })
      p.unref()
      return { success: true }
    } else {
      return { success: false, error: 'Unreal Engine or AtlasStudioOffice.uproject not found' }
    }
  } catch (err) {
    log('Failed to launch unreal:', err)
    return { success: false, error: err.message }
  }
})
