import { Terminal } from "../../vendor/xterm/xterm.js";
import { FitAddon } from "../../vendor/xterm/addon-fit.js";
import {
  readAgentSshTerminal,
  resizeAgentSshTerminal,
  writeAgentSshTerminal,
} from "../agent/agent-api.js";
import { escapeHtml } from "../../shared/ui.js";

const encoder = new TextEncoder();

export function mountSshTerminal(root, session, onClose, options = {}) {
  const mountId = crypto.randomUUID();
  root.dataset.terminalMountId = mountId;
  root.innerHTML = `
    <section class="ssh-terminal-shell">
      <header>
        <span><strong>${escapeHtml(session.name)}</strong><small>${escapeHtml(session.username)}@${escapeHtml(session.host)}:${session.port}</small></span>
        <button type="button" data-action="close-terminal" title="${options.detached ? "Close terminal window" : "Detach terminal to a separate window"}" aria-label="${options.detached ? "Close terminal window" : "Detach terminal to a separate window"}">${options.detached ? "Close" : "Detach"}</button>
      </header>
      <div class="ssh-terminal" data-role="ssh-terminal"></div>
    </section>`;

  const host = root.querySelector('[data-role="ssh-terminal"]');
  const style = getComputedStyle(document.documentElement);
  const terminal = new Terminal({
    cursorBlink: true,
    convertEol: false,
    fontFamily: "SFMono-Regular, Consolas, 'Liberation Mono', monospace",
    fontSize: 13,
    lineHeight: 1.18,
    scrollback: 10000,
    theme: {
      background: style.getPropertyValue("--surface").trim() || "#081522",
      foreground: style.getPropertyValue("--text-primary").trim() || "#dce9f5",
      cursor: style.getPropertyValue("--accent-bright").trim() || "#5ee7ef",
      selectionBackground: style.getPropertyValue("--accent-soft").trim() || "#19394d",
    },
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(host);

  let disposed = false;
  let reading = false;
  let offset = null;
  let resizeTimer = null;
  const resize = () => {
    if (disposed || !host.isConnected) return;
    fit.fit();
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      resizeAgentSshTerminal(session.id, terminal.cols, terminal.rows).catch(() => {});
    }, 80);
  };
  const observer = new ResizeObserver(resize);
  observer.observe(host);
  requestAnimationFrame(() => {
    resize();
    terminal.focus();
  });

  const input = terminal.onData((data) => {
    writeAgentSshTerminal(session.id, encoder.encode(data)).catch((cause) => {
      terminal.write(`\r\n\x1b[31m${String(cause)}\x1b[0m\r\n`);
    });
  });

  const dispose = () => {
    if (disposed) return;
    disposed = true;
    window.clearTimeout(resizeTimer);
    observer.disconnect();
    input.dispose();
    terminal.dispose();
  };
  root.querySelector('[data-action="close-terminal"]').addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      if (options.onDetach) await options.onDetach(session);
      dispose();
      await onClose();
    } catch (cause) {
      button.disabled = false;
      terminal.write(`\r\n\x1b[31mCould not detach terminal: ${String(cause)}\x1b[0m\r\n`);
    }
  });

  const poll = async () => {
    if (disposed || !root.isConnected || root.dataset.terminalMountId !== mountId) {
      dispose();
      return;
    }
    if (!reading) {
      reading = true;
      try {
        const chunk = await readAgentSshTerminal(session.id, offset);
        offset = chunk.nextOffset;
        if (chunk.truncated) terminal.write("\r\n\x1b[33mEarlier terminal output exceeded the 4 MB buffer.\x1b[0m\r\n");
        if (chunk.data.length) terminal.write(Uint8Array.from(chunk.data));
        if (chunk.closed) {
          terminal.write("\r\n\x1b[90mSSH session closed.\x1b[0m\r\n");
          return;
        }
      } catch (cause) {
        terminal.write(`\r\n\x1b[31m${String(cause)}\x1b[0m\r\n`);
        return;
      } finally {
        reading = false;
      }
    }
    window.setTimeout(poll, 50);
  };
  poll();
}
