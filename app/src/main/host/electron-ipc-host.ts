import type { BrowserWindow, IpcMain } from 'electron';

export interface IpcHost {
  handle<Args extends unknown[]>(channel: string, handler: (...args: Args) => unknown): void;
  send(channel: string, ...args: unknown[]): void;
}

export function createElectronIpcHost(deps: {
  ipcMain: IpcMain;
  getMainWindow: () => BrowserWindow | null;
}): IpcHost {
  const { ipcMain, getMainWindow } = deps;

  return {
    handle<Args extends unknown[]>(channel: string, handler: (...args: Args) => unknown) {
      ipcMain.handle(channel, (_event, ...args: unknown[]) => handler(...(args as Args)));
    },
    send(channel, ...args) {
      const mainWindow = getMainWindow();
      if (!mainWindow || mainWindow.isDestroyed()) {
        return;
      }

      const webContents = mainWindow.webContents;
      if (!webContents || webContents.isDestroyed()) {
        return;
      }

      try {
        if (webContents.getURL()) {
          webContents.send(channel, ...args);
        }
      } catch (error) {
        if (error instanceof Error && error.message.includes('Render frame was disposed')) {
          return;
        }
        console.error(`[ipc-host] failed to send ${channel}:`, error);
      }
    },
  };
}
