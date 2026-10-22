import { Terminal } from "../../vendor/xterm/xterm.js";
import { FitAddon } from "../../vendor/xterm/addon-fit.js";
import {
  disconnectAgentSshSession,
  readAgentSshTerminal,
  resizeAgentSshTerminal,
  writeAgentSshTerminal,
} from "../agent/agent-api.js";
import { escapeHtml } from "../../shared/ui.js";
import { configureTerminalKeyboard } from "./terminal-keyboard.js";

const encoder = new TextEncoder();

export function mountSshTerminal(root, session, onClose, options = {}) {
  const transport = options.transport ?? { read: readAgentSshTerminal, write: writeAgentSshTerminal, resize: resizeAgentSshTerminal, close: disconnectAgentSshSession };
  const local = options.local === true;
  const mountId = crypto.randomUUID();
  root.dataset.terminalMountId = mountId;
  root.innerHTML = `
    <section class="ssh-terminal-shell${local ? " is-local-terminal" : options.tabbed ? " is-tabbed-terminal" : ""}">
      <header>
        ${local ? "" : `<span>${options.tabbed ? "" : `<strong>${escapeHtml(session.name)}</strong>`}<small>${escapeHtml(session.username)}@${escapeHtml(session.host)}:${session.port}</small></span>`}
        <nav class="ssh-terminal-actions" aria-label="${local ? "Local terminal actions" : "SSH terminal actions"}">
          ${options.detached || local ? "" : '<button type="button" data-action="detach-terminal" data-tooltip="Detach terminal to a separate window" aria-label="Detach terminal to a separate window"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 4h6v6M20 4l-9 9M11 5H5v14h14v-6"/></svg></button>'}
          <button type="button" data-action="close-terminal" data-tooltip="${local ? "Close local terminal" : "Disconnect SSH and close terminal"}" aria-label="${local ? "Close local terminal" : "Disconnect SSH and close terminal"}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg></button>
        </nav>
      </header>
      <div class="ssh-terminal" data-role="ssh-terminal"><div class="ssh-terminal-viewport" data-role="ssh-terminal-viewport"></div></div>
    </section>`;

  const host = root.querySelector('[data-role="ssh-terminal"]');
  const viewport = root.querySelector('[data-role="ssh-terminal-viewport"]');
  if (options.theme) {
    host.style.backgroundColor = options.theme.background;
    root.querySelector(".ssh-terminal-shell").style.setProperty("--terminal-foreground", options.theme.foreground);
  }
  const style = getComputedStyle(document.documentElement);
  const terminal = new Terminal({
    cursorBlink: true,
    convertEol: false,
    fontFamily: "SFMono-Regular, Consolas, 'Liberation Mono', monospace",
    fontSize: 13,
    lineHeight: 1.18,
    scrollback: 10000,
    theme: options.theme ?? {
      background: style.getPropertyValue("--surface").trim() || "#081522",
      foreground: style.getPropertyValue("--text-primary").trim() || "#dce9f5",
      cursor: style.getPropertyValue("--accent-bright").trim() || "#5ee7ef",
      selectionBackground: style.getPropertyValue("--accent-soft").trim() || "#19394d",
    },
  });
  const fit = new FitAddon();
  configureTerminalKeyboard(terminal);
  terminal.loadAddon(fit);
  // FitAddon measures its parent's computed height, including border-box
  // padding. Use an unpadded viewport so the last row fits inside the shell.
  terminal.open(viewport);

  let disposed = false;
  let reading = false;
  let disconnected = false;
  let closing = false;
  let offset = null;
  let resizeTimer = null;
  let pollTimer = null;
  const resize = () => {
    if (disposed || !viewport.isConnected || !viewport.clientWidth || !viewport.clientHeight) return;
    fit.fit();
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      transport.resize(session.id, terminal.cols, terminal.rows).catch(() => {});
    }, 80);
  };
  const observer = new ResizeObserver(resize);
  observer.observe(viewport);
  requestAnimationFrame(() => {
    resize();
    if (!root.closest("[hidden]")) terminal.focus();
  });

  const markClosed = () => {
    if (disposed || disconnected) return;
    disconnected = true;
    const detach = root.querySelector('[data-action="detach-terminal"]');
    if (detach) detach.disabled = true;
    window.clearTimeout(pollTimer);
    options.onDisconnected?.(session);
    terminal.write(`\r\n\x1b[90m${local ? "Local terminal" : "SSH session"} closed.\x1b[0m\r\n`);
  };
  const input = terminal.onData((data) => {
    if (disposed || disconnected || closing) return;
    transport.write(session.id, encoder.encode(data)).catch((cause) => {
      if (disposed || disconnected) return;
      if (!local && /SSH session was not found|SSH terminal is closed/.test(String(cause))) { markClosed(); return; }
      terminal.write(`\r\n\x1b[31m${String(cause)}\x1b[0m\r\n`);
    });
  });

  const dispose = () => {
    if (disposed) return;
    disposed = true;
    window.clearTimeout(resizeTimer);
    window.clearTimeout(pollTimer);
    observer.disconnect();
    input.dispose();
    terminal.dispose();
  };
  const actionButtons = root.querySelectorAll(".ssh-terminal-actions button");
  const setBusy = (busy) => {
    closing = busy;
    actionButtons.forEach((button) => { button.disabled = busy; });
  };
  root.querySelector('[data-action="detach-terminal"]')?.addEventListener("click", async () => {
    if (closing || disposed) return;
    setBusy(true);
    try {
      if (options.onDetach) await options.onDetach(session);
      await onClose();
      dispose();
    } catch (cause) {
      setBusy(false);
      if (!disposed) terminal.write(`\r\n\x1b[31mCould not detach terminal: ${String(cause)}\x1b[0m\r\n`);
    }
  });
  root.querySelector('[data-action="close-terminal"]').addEventListener("click", async () => {
    if (closing || disposed) return;
    setBusy(true);
    try {
      if (!disconnected) {
        if (options.onDisconnect) await options.onDisconnect(session);
        else await transport.close(session.id);
        disconnected = true;
      }
      await onClose();
      dispose();
    } catch (cause) {
      setBusy(false);
      if (!disposed) terminal.write(`\r\n\x1b[31mCould not close terminal: ${String(cause)}\x1b[0m\r\n`);
    }
  });

  const poll = async () => {
    if (disconnected) return;
    if (disposed || !root.isConnected || root.dataset.terminalMountId !== mountId) {
      dispose();
      return;
    }
    if (root.closest("[hidden]")) { pollTimer = window.setTimeout(poll, 500); return; }
    if (!reading) {
      reading = true;
      try {
        const chunk = await transport.read(session.id, offset);
        if (disposed) return;
        offset = chunk.nextOffset;
        if (chunk.truncated) terminal.write("\r\n\x1b[33mEarlier terminal output exceeded the 4 MB buffer.\x1b[0m\r\n");
        if (chunk.data.length) terminal.write(Uint8Array.from(chunk.data));
        if (chunk.closed) { markClosed(); return; }
      } catch (cause) {
        if (disposed) return;
        if (!local && /SSH session was not found|SSH terminal is closed/.test(String(cause))) { markClosed(); return; }
        options.onDisconnected?.(session);
        terminal.write(`\r\n\x1b[31m${String(cause)}\x1b[0m\r\n`);
        return;
      } finally {
        reading = false;
      }
    }
    pollTimer = window.setTimeout(poll, 50);
  };
  poll();
  return { dispose, activate() { if (!disposed) { resize(); terminal.focus(); } } };
}
