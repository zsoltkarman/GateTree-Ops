import { listAgentSshSessions } from "../agent/agent-api.js";
import { mountSshTerminal } from "./terminal-view.js";

const { WebviewWindow, getCurrentWebviewWindow } = window.__TAURI__.webviewWindow;

export async function openDetachedTerminalWindow(session) {
  const label = `terminal-${session.id}`;
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.show();
    await existing.setFocus();
    return;
  }

  const detached = new WebviewWindow(label, {
    url: `index.html?detachedTerminal=${encodeURIComponent(session.id)}`,
    title: `PHAROS · ${session.name}`,
    width: 960,
    height: 620,
    minWidth: 560,
    minHeight: 360,
    resizable: true,
    focus: true,
  });
  await new Promise((resolve, reject) => {
    detached.once("tauri://created", resolve);
    detached.once("tauri://error", (event) => reject(new Error(String(event.payload))));
  });
}

export async function focusDetachedTerminalWindow(session) {
  const detached = await WebviewWindow.getByLabel(`terminal-${session.id}`);
  if (!detached) return false;
  await detached.show();
  await detached.unminimize();
  await detached.setFocus();
  return true;
}

export async function mountDetachedTerminalWindow(root, sessionId) {
  document.body.classList.add("detached-terminal-window");
  root.innerHTML = '<div class="detached-terminal-loading">Opening SSH terminal…</div>';
  try {
    const sessions = await listAgentSshSessions();
    const session = sessions.find((item) => item.id === sessionId);
    if (!session) throw new Error("This SSH session is no longer running.");
    root.innerHTML = '<div class="ssh-terminal-mount detached-terminal-mount" data-role="detached-terminal-mount"></div>';
    mountSshTerminal(
      root.querySelector('[data-role="detached-terminal-mount"]'),
      session,
      () => getCurrentWebviewWindow().close(),
      { detached: true },
    );
  } catch (cause) {
    root.innerHTML = `<div class="detached-terminal-error">${escapeText(cause)}</div>`;
  }
}

function escapeText(value) {
  const element = document.createElement("span");
  element.textContent = String(value);
  return element.innerHTML;
}
