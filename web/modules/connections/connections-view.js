import { escapeHtml } from "../../shared/ui.js";
import { createModuleRoot } from "../../shared/module-root.js";
import {
  connectAgentSshConnection,
  inspectAgentSshHostKey,
  listAgentSshSessions,
  openUrlConnection,
  trustAgentSshHostKey,
} from "../agent/agent-api.js";
import { getVaults } from "../vaults/vaults-api.js";
import { mountSshTerminal } from "./terminal-view.js";
import { openDetachedTerminalWindow } from "./detached-terminal-window.js";
import {
  createConnectionFolder,
  createConnectionItem,
  deleteConnectionFolder,
  listConnectionFolders,
  listConnectionItems,
  moveConnectionItem,
  updateConnectionItem,
  moveConnectionFolder,
  renameConnectionFolder,
} from "./connections-api.js";

const BROWSER_WIDTH_KEY = "pharos.connections.browserWidth";

export async function mountConnectionsView(contentRoot, options = {}) {
  const root = createModuleRoot(contentRoot, options.navigationRoot);
  const state = {
    folders: [],
    connections: [],
    vaultEntries: [],
    selectedId: null,
    expandedIds: new Set(),
    createParentId: undefined,
    renamingId: null,
    pointerDrag: null,
    pointerConnectionDrag: null,
    connectionClickTimer: null,
    suppressClick: false,
    creatingConnection: null,
    selectedConnectionId: null,
    terminalSession: options.terminalSession ?? null,
    sshSessions: [],
    error: "",
    browserWidth: storedBrowserWidth(),
    navigationRoot: options.navigationRoot ?? null,
  };
  root.innerHTML = '<div class="connections-loading">Loading connection folders…</div>';
  try {
    const [folders, connections, vaults, sshSessions] = await Promise.all([
      listConnectionFolders(),
      listConnectionItems(),
      getVaults(),
      listAgentSshSessions().catch(() => []),
    ]);
    state.folders = folders;
    state.connections = connections;
    state.vaultEntries = vaults.entries;
    state.sshSessions = sshSessions;
    if (state.terminalSession) {
      state.selectedConnectionId = state.terminalSession.connectionId;
    }
    state.selectedId = state.folders[0]?.id ?? null;
    state.folders.filter((folder) => !folder.parentId).forEach((folder) => state.expandedIds.add(folder.id));
    render(root, state);
  } catch (cause) {
    root.innerHTML = `<div class="connections-error">${escapeHtml(cause)}</div>`;
  }
}

function render(root, state) {
  const roots = childrenOf(state, null);
  const selected = folderById(state, state.selectedId);
  const selectedConnection = state.connections.find((item) => item.id === state.selectedConnectionId) ?? null;
  root.innerHTML = `
    <div class="connections-module${state.navigationRoot ? " is-main-navigation-layout" : ""}" style="--connections-browser-width:${state.browserWidth}px">
      <aside class="connections-browser">
        ${state.createParentId !== undefined ? createForm(state) : ""}
        <div class="connection-folder-tree">
          ${roots.map((folder) => folderNode(folder, state, 0)).join("")}
          ${state.folders.length ? "" : '<p class="connections-empty-tree">Create your first folder</p>'}
        </div>
      </aside>
      <div class="connections-resizer" data-role="connections-resizer" role="separator" aria-label="Resize connection folders" aria-orientation="vertical" tabindex="0"></div>
      <section class="connections-workspace${state.terminalSession ? " has-terminal" : ""}">
        ${state.terminalSession ? '<div class="ssh-terminal-mount" data-role="ssh-terminal-mount"></div>' : state.creatingConnection ? connectionForm(state) : selectedConnection ? connectionPanel(selectedConnection, state) : selected ? folderPanel(selected, state) : emptyPanel()}
      </section>
      <div class="connections-toast" ${state.error ? "" : "hidden"}>${escapeHtml(state.error)}</div>
    </div>`;
  bindEvents(root, state);
  if (state.navigationRoot) root.mountNavigation(".connections-browser");
  const terminalMount = root.querySelector('[data-role="ssh-terminal-mount"]');
  if (terminalMount) {
    mountSshTerminal(
      terminalMount,
      state.terminalSession,
      () => {
        state.terminalSession = null;
        render(root, state);
      },
      { onDetach: openDetachedTerminalWindow },
    );
  }
}

function createForm(state) {
  const parent = folderById(state, state.createParentId);
  return `
    <form class="connection-folder-form" data-role="create-folder-form">
      <small>${parent ? `Inside ${escapeHtml(parent.name)}` : "Root folder"}</small>
      <span>
        <input data-role="new-folder-name" maxlength="120" placeholder="Folder name" autocomplete="off" />
        <button type="submit" title="Create folder" aria-label="Create folder">${icon("check")}</button>
        <button type="button" data-action="cancel-create" title="Cancel" aria-label="Cancel">${icon("close")}</button>
      </span>
    </form>`;
}

function folderNode(folder, state, depth) {
  const children = childrenOf(state, folder.id);
  const connections = connectionsInFolder(state, folder.id);
  const hasContent = children.length || connections.length;
  const expanded = state.expandedIds.has(folder.id);
  return `
    <div class="connection-tree-branch">
      <div class="connection-folder-row${state.selectedId === folder.id ? " is-active" : ""}" style="--folder-depth:${depth}" data-folder-id="${folder.id}">
        <button class="connection-folder-toggle${hasContent ? "" : " is-empty"}" data-action="toggle" data-folder-id="${folder.id}" ${hasContent ? "" : "disabled"} aria-label="${expanded ? "Collapse" : "Expand"} ${escapeHtml(folder.name)}">${icon("chevron")}</button>
        ${state.renamingId === folder.id ? `
          <form class="connection-inline-rename" data-role="rename-form" data-folder-id="${folder.id}">
            ${icon("folder")}
            <input data-role="rename-input" value="${escapeHtml(folder.name)}" maxlength="120" autocomplete="off" />
          </form>` : `
          <button class="connection-folder-main" data-action="select" data-folder-id="${folder.id}">${icon("folder")}<span>${escapeHtml(folder.name)}</span></button>`}
      </div>
      <div class="connection-folder-children" ${expanded && children.length ? "" : "hidden"}>
        ${children.map((child) => folderNode(child, state, depth + 1)).join("")}
      </div>
      <div class="connection-items" style="--connection-indent:${(depth + 1) * 17 + 24}px" ${expanded && connections.length ? "" : "hidden"}>
        ${connections.map((item) => `<button class="connection-item-row${state.selectedConnectionId === item.id ? " is-active" : ""}" data-action="select-connection" data-connection-id="${item.id}">${connectionIcon(item.connectionType)}<span>${escapeHtml(item.name)}</span>${state.sshSessions.some((session) => session.connectionId === item.id) ? '<i class="connection-session-indicator" title="SSH session running in Agent" aria-label="SSH session running in Agent"></i>' : ""}</button>`).join("")}
      </div>
    </div>`;
}

function folderPanel(folder, state) {
  const children = childrenOf(state, folder.id);
  const connections = connectionsInFolder(state, folder.id);
  return `
    <div class="connection-folder-panel">
      ${icon("folder-large")}
      <strong>${escapeHtml(folder.name)}</strong>
      <span>${escapeHtml(folderPath(state, folder.id))}</span>
      <small>${children.length} subfolder${children.length === 1 ? "" : "s"} · ${connections.length} connection${connections.length === 1 ? "" : "s"}</small>
    </div>`;
}

function connectionForm(state) {
  const { folderId, type, connectionId } = state.creatingConnection;
  const folder = folderById(state, folderId);
  const existing = state.connections.find((item) => item.id === connectionId) ?? null;
  const isUrl = type === "url";
  const isSsh = type === "ssh";
  const authentication = existing?.authentication ?? "password";
  return `
    <form class="connection-editor" data-role="connection-form">
      <span class="connection-editor-icon">${connectionIcon(type)}</span>
      <strong>${existing ? "Edit" : "New"} ${type.toUpperCase()} connection</strong>
      <small>${escapeHtml(folderPath(state, folderId))}</small>
      <label><span>Name</span><input data-role="connection-name" maxlength="120" autocomplete="off" required value="${escapeHtml(existing?.name ?? "")}" /></label>
      <label><span>${isUrl ? "URL" : "Host"}</span><input data-role="connection-target" autocomplete="off" spellcheck="false" required value="${escapeHtml(existing?.target ?? "")}" /></label>
      ${isUrl ? "" : `<label><span>Port</span><input data-role="connection-port" type="number" min="1" max="65535" value="${existing?.port ?? (type === "ssh" ? 22 : 3389)}" /></label>`}
      ${isSsh ? `
        <label><span>SSH engine</span><select data-role="connection-ssh-transport">
          <option value="embedded"${(existing?.sshTransport ?? "embedded") === "embedded" ? " selected" : ""}>PHAROS embedded SSH</option>
          <option value="openssh"${existing?.sshTransport === "openssh" ? " selected" : ""}>Local OpenSSH config (ProxyJump / MFA)</option>
        </select></label>
        <label><span>Username</span><input data-role="connection-username" maxlength="255" autocomplete="off" value="${escapeHtml(existing?.username ?? "")}" /></label>
        <label><span>Authentication</span><select data-role="connection-authentication">
          ${authenticationOption("password", "Password", authentication)}
          ${authenticationOption("private-key", "Private key", authentication)}
          ${authenticationOption("keyboard-interactive", "Keyboard interactive", authentication)}
          ${authenticationOption("ssh-agent", "SSH agent", authentication)}
        </select></label>
        <label><span>Credential reference</span><select data-role="connection-credential-reference">
          <option value="">None</option>
          ${state.vaultEntries.map((entry) => `<option value="${escapeHtml(entry.id)}"${entry.id === existing?.credentialReference ? " selected" : ""}>${escapeHtml(entry.name)}</option>`).join("")}
        </select></label>` : ""}
      <label><span>Tag</span><input data-role="connection-tag" maxlength="120" autocomplete="off" value="${escapeHtml(existing?.tag ?? "")}" /></label>
      <div><button class="connection-primary-action" type="submit">Save</button><button type="button" data-action="cancel-connection">Cancel</button></div>
    </form>`;
}

function connectionPanel(connection, state) {
  const credential = state.vaultEntries.find((entry) => entry.id === connection.credentialReference);
  const activeSession = state.sshSessions.find((session) => session.connectionId === connection.id);
  const endpoint = connection.connectionType === "ssh" && connection.username
    ? `${connection.username}@${connection.target}`
    : connection.target;
  const details = [
    connection.connectionType === "ssh" && connection.sshTransport === "openssh" ? "OpenSSH config" : connection.connectionType === "ssh" ? authenticationLabel(connection.authentication) : connection.connectionType.toUpperCase(),
    credential?.name,
    connection.tag,
  ].filter(Boolean).map(escapeHtml).join(" · ");
  return `
    <div class="connection-folder-panel">
      ${connectionIcon(connection.connectionType)}
      <strong>${escapeHtml(connection.name)}</strong>
      <span>${escapeHtml(endpoint)}${connection.port ? `:${connection.port}` : ""}</span>
      <small>${details}</small>
      ${activeSession ? '<span class="connection-running-status"><i></i>Running in Agent</span>' : ""}
      ${activeSession ? `<button class="connection-open-action" data-action="attach-active-session" data-session-id="${activeSession.id}">Open</button>` : ""}
      <button class="connection-secondary-action" data-action="edit-connection" data-connection-id="${connection.id}">Edit</button>
      ${connection.connectionType === "url" ? `<button class="connection-open-action" data-action="open-url" data-connection-id="${connection.id}">Open</button>` : ""}
    </div>`;
}

function emptyPanel() {
  return `<div class="connections-empty-panel">${icon("folder-large")}<span>Select or create a folder.</span></div>`;
}

function bindEvents(root, state) {
  bindResizer(root, state);
  bindContextMenus(root, state);
  bindFolderDragging(root, state);
  bindConnectionDragging(root, state);
  root.querySelector('[data-action="cancel-create"]')?.addEventListener("click", () => {
    state.createParentId = undefined;
    render(root, state);
  });
  root.querySelector('[data-role="create-folder-form"]')?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const name = root.querySelector('[data-role="new-folder-name"]').value.trim();
    if (!name) return;
    const parentId = state.createParentId;
    await runAction(root, state, async () => {
      const folder = await createConnectionFolder(name, parentId);
      state.folders.push(folder);
      state.selectedId = folder.id;
      state.createParentId = undefined;
      if (parentId) state.expandedIds.add(parentId);
    });
  });
  root.querySelectorAll('[data-action="toggle"]').forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.folderId;
      if (state.expandedIds.has(id)) state.expandedIds.delete(id);
      else state.expandedIds.add(id);
      render(root, state);
    });
  });
  root.querySelectorAll('[data-action="select"]').forEach((button) => {
    button.addEventListener("click", () => {
      if (state.suppressClick) return;
      state.selectedId = button.dataset.folderId;
      state.selectedConnectionId = null;
      state.creatingConnection = null;
      state.terminalSession = null;
      render(root, state);
    });
  });
  root.querySelector('[data-role="rename-form"]')?.addEventListener("submit", async (event) => {
    event.preventDefault();
    await commitRename(root, state);
  });
  const renameInput = root.querySelector('[data-role="rename-input"]');
  renameInput?.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    state.renamingId = null;
    render(root, state);
  });
  renameInput?.addEventListener("blur", () => {
    if (state.renamingId) commitRename(root, state);
  });
  root.querySelectorAll('[data-action="select-connection"]').forEach((button) => {
    button.addEventListener("click", () => {
      if (state.suppressClick) return;
      const connection = state.connections.find((item) => item.id === button.dataset.connectionId);
      const select = () => {
        state.connectionClickTimer = null;
        state.selectedConnectionId = connection.id;
        state.selectedId = connection.folderId;
        state.creatingConnection = null;
        state.terminalSession = null;
        render(root, state);
      };
      if (connection.connectionType === "url" || connection.connectionType === "ssh") {
        window.clearTimeout(state.connectionClickTimer);
        state.connectionClickTimer = window.setTimeout(select, 240);
      } else {
        select();
      }
    });
    button.addEventListener("dblclick", async (event) => {
      if (state.suppressClick) return;
      const connection = state.connections.find((item) => item.id === button.dataset.connectionId);
      if (!connection || !["url", "ssh"].includes(connection.connectionType)) return;
      event.preventDefault();
      window.clearTimeout(state.connectionClickTimer);
      state.connectionClickTimer = null;
      await runAction(root, state, async () => {
        state.selectedConnectionId = connection.id;
        state.selectedId = connection.folderId;
        state.creatingConnection = null;
        if (connection.connectionType === "url") {
          await openUrlConnection(connection.id);
          return;
        }
        const sessions = await listAgentSshSessions();
        state.sshSessions = sessions;
        const session = sessions.find((item) => item.connectionId === connection.id);
        if (session) {
          state.terminalSession = session;
          return;
        }
        if (connection.sshTransport === "openssh") {
          state.terminalSession = await connectAgentSshConnection(connection.id, {
            username: connection.username ?? "",
          });
          state.sshSessions.push(state.terminalSession);
          return;
        }
        const port = connection.port ?? 22;
        let inspection;
        try {
          inspection = await inspectAgentSshHostKey(connection.target, port);
        } catch (cause) {
          if (!String(cause).includes("did not provide a host key")) throw cause;
          const useOpenSsh = window.confirm(
            `${connection.target} is not directly reachable by embedded SSH.\n\n` +
            "Use the local OpenSSH configuration for this connection? " +
            "This supports ProxyJump chains and interactive MFA.",
          );
          if (!useOpenSsh) return;
          const updated = await updateConnectionItem(
            connection.id,
            connection.name,
            connection.target,
            connection.port,
            connection.tag,
            connection.username,
            connection.authentication,
            connection.credentialReference,
            "openssh",
          );
          Object.assign(connection, updated);
          state.terminalSession = await connectAgentSshConnection(connection.id, {
            username: connection.username ?? "",
          });
          state.sshSessions.push(state.terminalSession);
          return;
        }
        if (inspection.state !== "trusted") {
          const confirmed = inspection.state === "changed"
            ? window.confirm(
                `WARNING: The SSH host key for ${connection.target}:${port} changed.\n\n` +
                `Previously trusted:\n${inspection.trustedFingerprint ?? "Unknown"}\n\n` +
                `Current:\n${inspection.fingerprint}\n\nReplace the trusted key and continue?`,
              )
            : window.confirm(
                `First connection to ${connection.target}:${port}.\n\n` +
                `Host key fingerprint:\n${inspection.fingerprint}\n\nTrust this host and continue?`,
              );
          if (!confirmed) return;
          await trustAgentSshHostKey(connection.target, port, inspection.publicKey);
        }
        const connectedSession = await connectSshWithDialog(connection);
        if (connectedSession) {
          state.terminalSession = connectedSession;
          state.sshSessions.push(connectedSession);
        }
      });
    });
  });
  root.querySelector('[data-action="cancel-connection"]')?.addEventListener("click", () => {
    state.creatingConnection = null;
    render(root, state);
  });
  root.querySelector('[data-role="connection-form"]')?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const { folderId, type } = state.creatingConnection;
    const name = root.querySelector('[data-role="connection-name"]').value.trim();
    const target = root.querySelector('[data-role="connection-target"]').value.trim();
    const portInput = root.querySelector('[data-role="connection-port"]');
    const port = portInput ? Number(portInput.value) : null;
    const tag = root.querySelector('[data-role="connection-tag"]').value.trim();
    const usernameInput = root.querySelector('[data-role="connection-username"]');
    const username = usernameInput?.value.trim() || null;
    const authentication = root.querySelector('[data-role="connection-authentication"]')?.value || null;
    const credentialReference = root.querySelector('[data-role="connection-credential-reference"]')?.value || null;
    const sshTransport = root.querySelector('[data-role="connection-ssh-transport"]')?.value || null;
    if (!name || !target) {
      state.error = "Name and target are required.";
      render(root, state);
      return;
    }
    if (portInput && (!Number.isInteger(port) || port < 1 || port > 65535)) {
      state.error = "Port must be between 1 and 65535.";
      render(root, state);
      return;
    }
    await runAction(root, state, async () => {
      const connectionId = state.creatingConnection.connectionId;
      const connection = connectionId
        ? await updateConnectionItem(connectionId, name, target, port, tag, username, authentication, credentialReference, sshTransport)
        : await createConnectionItem(folderId, type, name, target, port, tag, username, authentication, credentialReference, sshTransport);
      const index = state.connections.findIndex((item) => item.id === connection.id);
      if (index >= 0) state.connections[index] = connection;
      else state.connections.push(connection);
      state.selectedConnectionId = connection.id;
      state.selectedId = folderId;
      state.creatingConnection = null;
      state.expandedIds.add(folderId);
    });
  });
  root.querySelector('[data-action="open-url"]')?.addEventListener("click", async (event) => {
    await runAction(root, state, async () => {
      await openUrlConnection(event.currentTarget.dataset.connectionId);
    });
  });
  root.querySelector('[data-action="edit-connection"]')?.addEventListener("click", (event) => {
    const connection = state.connections.find((item) => item.id === event.currentTarget.dataset.connectionId);
    state.terminalSession = null;
    state.creatingConnection = {
      folderId: connection.folderId,
      type: connection.connectionType,
      connectionId: connection.id,
    };
    render(root, state);
    root.querySelector('[data-role="connection-name"]')?.focus();
  });
  root.querySelector('[data-action="attach-active-session"]')?.addEventListener("click", (event) => {
    const session = state.sshSessions.find((item) => item.id === event.currentTarget.dataset.sessionId);
    if (!session) return;
    state.terminalSession = session;
    render(root, state);
  });
}

function connectSshWithDialog(connection) {
  const privateKey = connection.authentication === "private-key";
  const sshAgent = connection.authentication === "ssh-agent";
  const dialog = document.createElement("dialog");
  dialog.className = "ssh-auth-dialog";
  dialog.innerHTML = `
    <form>
      <strong>Connect to ${escapeHtml(connection.name)}</strong>
      <small>${escapeHtml(connection.target)}:${connection.port ?? 22}</small>
      <label><span>Username</span><input data-role="ssh-username" required autocomplete="username" value="${escapeHtml(connection.username ?? "")}" /></label>
      ${sshAgent ? "" : privateKey ? `
        <label><span>Private key</span><textarea data-role="ssh-private-key" required spellcheck="false"></textarea></label>
        <label><span>Passphrase</span><input data-role="ssh-passphrase" type="password" autocomplete="off" /></label>` : `
        <label><span>${connection.authentication === "keyboard-interactive" ? "Response" : "Password"}</span><input data-role="ssh-credential" type="password" required autocomplete="off" /></label>`}
      <p class="ssh-auth-error" data-role="ssh-auth-error" hidden></p>
      <div><button type="button" data-action="cancel-ssh">Cancel</button><button class="connection-primary-action" type="submit">Connect</button></div>
    </form>`;
  document.body.append(dialog);
  return new Promise((resolve) => {
    const close = (result) => {
      dialog.close();
      dialog.remove();
      resolve(result);
    };
    dialog.querySelector('[data-action="cancel-ssh"]').addEventListener("click", () => close(null));
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      close(null);
    });
    dialog.querySelector("form").addEventListener("submit", async (event) => {
      event.preventDefault();
      const submit = dialog.querySelector('[type="submit"]');
      const error = dialog.querySelector('[data-role="ssh-auth-error"]');
      const authentication = sshAgent
        ? { username: dialog.querySelector('[data-role="ssh-username"]').value.trim() }
        : privateKey
        ? {
            username: dialog.querySelector('[data-role="ssh-username"]').value.trim(),
            privateKey: dialog.querySelector('[data-role="ssh-private-key"]').value,
            passphrase: dialog.querySelector('[data-role="ssh-passphrase"]').value || null,
          }
        : {
            username: dialog.querySelector('[data-role="ssh-username"]').value.trim(),
            credential: dialog.querySelector('[data-role="ssh-credential"]').value,
          };
      submit.disabled = true;
      error.hidden = true;
      try {
        close(await connectAgentSshConnection(connection.id, authentication));
      } catch (cause) {
        error.textContent = String(cause);
        error.hidden = false;
        submit.disabled = false;
        dialog.querySelector('[data-role="ssh-credential"]')?.select();
      }
    });
    dialog.showModal();
    dialog.querySelector("input, textarea")?.focus();
  });
}

function bindConnectionDragging(root, state) {
  const clearTargets = () => root.querySelectorAll(".is-drop-target").forEach((item) => item.classList.remove("is-drop-target"));
  const targetAt = (x, y, item) => {
    const row = document.elementFromPoint(x, y)?.closest(".connection-folder-row");
    if (!row || row.dataset.folderId === item.folderId) return null;
    return row;
  };
  const finish = async (button, event, cancelled = false) => {
    const drag = state.pointerConnectionDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const item = state.connections.find((connection) => connection.id === drag.connectionId);
    const target = drag.active && !cancelled ? targetAt(event.clientX, event.clientY, item) : null;
    if (button.hasPointerCapture(event.pointerId)) button.releasePointerCapture(event.pointerId);
    state.pointerConnectionDrag = null;
    button.classList.remove("is-dragging");
    clearTargets();
    if (!drag.active) return;
    event.preventDefault();
    window.setTimeout(() => { state.suppressClick = false; }, 0);
    if (!target) return;
    await runAction(root, state, async () => {
      Object.assign(item, await moveConnectionItem(item.id, target.dataset.folderId));
      state.selectedId = item.folderId;
      state.selectedConnectionId = item.id;
      state.expandedIds.add(item.folderId);
    });
  };
  root.querySelectorAll(".connection-item-row").forEach((button) => {
    button.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      state.pointerConnectionDrag = {
        pointerId: event.pointerId,
        connectionId: button.dataset.connectionId,
        startX: event.clientX,
        startY: event.clientY,
        active: false,
      };
      button.setPointerCapture(event.pointerId);
    });
    button.addEventListener("pointermove", (event) => {
      const drag = state.pointerConnectionDrag;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) >= 5) {
        drag.active = true;
        state.suppressClick = true;
        button.classList.add("is-dragging");
      }
      if (!drag.active) return;
      event.preventDefault();
      clearTargets();
      const item = state.connections.find((connection) => connection.id === drag.connectionId);
      targetAt(event.clientX, event.clientY, item)?.classList.add("is-drop-target");
    });
    button.addEventListener("pointerup", (event) => finish(button, event));
    button.addEventListener("pointercancel", (event) => finish(button, event, true));
  });
}

function bindContextMenus(root, state) {
  const tree = root.querySelector(".connection-folder-tree");
  tree.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    const connectionRow = event.target.closest(".connection-item-row");
    if (connectionRow) {
      const connection = state.connections.find((item) => item.id === connectionRow.dataset.connectionId);
      state.selectedId = connection.folderId;
      state.selectedConnectionId = connection.id;
      showConnectionContextMenu(root, state, connection, event.clientX, event.clientY);
      return;
    }
    const row = event.target.closest(".connection-folder-row");
    const folder = row ? folderById(state, row.dataset.folderId) : null;
    if (folder) {
      state.selectedId = folder.id;
      root.querySelector(".connection-folder-row.is-active")?.classList.remove("is-active");
      row.classList.add("is-active");
    }
    showContextMenu(root, state, folder, event.clientX, event.clientY);
  });
}

function showConnectionContextMenu(root, state, connection, x, y) {
  document.querySelector(".connection-context-menu")?.remove();
  const menu = document.createElement("div");
  menu.className = "connection-context-menu";
  menu.innerHTML = `
    <button data-context-action="copy-target">${icon("copy")}<span>Copy hostname / IP</span></button>
    <button data-context-action="edit">${icon("pencil")}<span>Edit</span></button>`;
  document.body.append(menu);
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - menu.offsetWidth - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - menu.offsetHeight - 8))}px`;
  const close = (event) => {
    if (event && menu.contains(event.target)) return;
    menu.remove();
    window.removeEventListener("pointerdown", close);
    window.removeEventListener("blur", close);
  };
  window.setTimeout(() => window.addEventListener("pointerdown", close), 0);
  window.addEventListener("blur", close);
  menu.querySelector('[data-context-action="copy-target"]').addEventListener("click", async () => {
    close();
    try {
      await copyText(connection.target);
    } catch (cause) {
      state.error = String(cause);
      render(root, state);
    }
  });
  menu.querySelector('[data-context-action="edit"]').addEventListener("click", () => {
    close();
    state.terminalSession = null;
    state.creatingConnection = {
      folderId: connection.folderId,
      type: connection.connectionType,
      connectionId: connection.id,
    };
    render(root, state);
    root.querySelector('[data-role="connection-name"]').focus();
  });
}

function showContextMenu(root, state, folder, x, y) {
  document.querySelector(".connection-context-menu")?.remove();
  const menu = document.createElement("div");
  menu.className = "connection-context-menu";
  menu.innerHTML = folder ? `
    <button data-context-action="new">${icon("folder-plus")}<span>New subfolder</span></button>
    <button data-context-action="rename">${icon("pencil")}<span>Rename</span></button>
    <button class="is-danger" data-context-action="delete">${icon("trash")}<span>Delete</span></button>
    <div class="connection-context-separator"></div>
    <div class="connection-context-submenu">
      <button>${icon("connection")}<span>New connection</span>${icon("chevron")}</button>
      <div class="connection-context-children">
        <button data-connection-type="ssh">${connectionIcon("ssh")}<span>SSH</span></button>
        <button data-connection-type="rdp">${connectionIcon("rdp")}<span>RDP</span></button>
        <button data-connection-type="url">${connectionIcon("url")}<span>URL</span></button>
      </div>
    </div>` : `
    <button data-context-action="new">${icon("folder-plus")}<span>New folder</span></button>`;
  document.body.append(menu);
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - menu.offsetWidth - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - menu.offsetHeight - 8))}px`;

  const close = () => {
    menu.remove();
    window.removeEventListener("pointerdown", closeOnOutside);
    window.removeEventListener("blur", close);
    window.removeEventListener("keydown", onKeydown);
  };
  const closeOnOutside = (event) => {
    if (!menu.contains(event.target)) close();
  };
  const onKeydown = (event) => {
    if (event.key === "Escape") close();
  };
  window.setTimeout(() => window.addEventListener("pointerdown", closeOnOutside), 0);
  window.addEventListener("blur", close);
  window.addEventListener("keydown", onKeydown);

  menu.querySelector('[data-context-action="new"]').addEventListener("click", () => {
    close();
    showCreate(root, state, folder?.id ?? null);
  });
  menu.querySelector('[data-context-action="rename"]')?.addEventListener("click", () => {
    close();
    beginRename(root, state, folder);
  });
  menu.querySelector('[data-context-action="delete"]')?.addEventListener("click", () => {
    close();
    deleteFolder(root, state, folder);
  });
  menu.querySelectorAll("[data-connection-type]").forEach((button) => {
    button.addEventListener("click", () => {
      close();
      state.selectedId = folder.id;
      state.selectedConnectionId = null;
      state.terminalSession = null;
      state.creatingConnection = { folderId: folder.id, type: button.dataset.connectionType };
      state.expandedIds.add(folder.id);
      render(root, state);
      root.querySelector('[data-role="connection-name"]').focus();
    });
  });
}

function bindFolderDragging(root, state) {
  const tree = root.querySelector(".connection-folder-tree");
  const clearTargets = () => root.querySelectorAll(".is-drop-target").forEach((item) => item.classList.remove("is-drop-target"));
  const targetAt = (x, y) => {
    const element = document.elementFromPoint(x, y);
    const row = element?.closest(".connection-folder-row");
    if (row) {
      if (canMoveTo(state, state.pointerDrag?.folderId, row.dataset.folderId)) {
        return { row, parentId: row.dataset.folderId };
      }
      return null;
    }
    if (element && tree.contains(element)) return { row: null, parentId: null };
    return null;
  };
  const finish = async (button, event, cancelled = false) => {
    const drag = state.pointerDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const target = drag.active && !cancelled ? targetAt(event.clientX, event.clientY) : null;
    if (button.hasPointerCapture(event.pointerId)) button.releasePointerCapture(event.pointerId);
    state.pointerDrag = null;
    drag.row.classList.remove("is-dragging");
    clearTargets();
    tree.classList.remove("is-drop-target");
    if (drag.active) {
      event.preventDefault();
      window.setTimeout(() => { state.suppressClick = false; }, 0);
      if (target) await moveFolder(root, state, drag.folderId, target.parentId);
    }
  };
  root.querySelectorAll(".connection-folder-main").forEach((button) => {
    button.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const row = button.closest(".connection-folder-row");
      state.pointerDrag = {
        pointerId: event.pointerId,
        folderId: button.dataset.folderId,
        startX: event.clientX,
        startY: event.clientY,
        active: false,
        row,
      };
      button.setPointerCapture(event.pointerId);
    });
    button.addEventListener("pointermove", (event) => {
      const drag = state.pointerDrag;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) >= 5) {
        drag.active = true;
        state.suppressClick = true;
        drag.row.classList.add("is-dragging");
      }
      if (!drag.active) return;
      event.preventDefault();
      clearTargets();
      tree.classList.remove("is-drop-target");
      const target = targetAt(event.clientX, event.clientY);
      if (target?.row) target.row.classList.add("is-drop-target");
      else if (target) tree.classList.add("is-drop-target");
    });
    button.addEventListener("pointerup", (event) => finish(button, event));
    button.addEventListener("pointercancel", (event) => finish(button, event, true));
  });
}

function canMoveTo(state, folderId, parentId) {
  return Boolean(folderId && parentId && folderId !== parentId && !descendantIds(state, folderId).includes(parentId));
}

async function moveFolder(root, state, folderId, parentId) {
  const folder = folderById(state, folderId);
  if (!folder || folder.parentId === parentId) return;
  await runAction(root, state, async () => {
    Object.assign(folder, await moveConnectionFolder(folder.id, parentId));
    if (parentId) state.expandedIds.add(parentId);
    state.selectedId = folder.id;
  });
}

function beginRename(root, state, folder) {
  state.renamingId = folder.id;
  render(root, state);
  const input = root.querySelector('[data-role="rename-input"]');
  input.focus();
  input.select();
}

async function commitRename(root, state) {
  const id = state.renamingId;
  const input = root.querySelector('[data-role="rename-input"]');
  if (!id || !input) return;
  const folder = folderById(state, id);
  const name = input.value.trim();
  if (!name || name === folder.name) {
    state.renamingId = null;
    render(root, state);
    return;
  }
  state.renamingId = null;
  await runAction(root, state, async () => Object.assign(folder, await renameConnectionFolder(folder.id, name)));
}

async function deleteFolder(root, state, folder) {
  const count = descendantIds(state, folder.id).length;
  const suffix = count ? ` and its ${count} subfolder${count === 1 ? "" : "s"}` : "";
  if (!window.confirm(`Delete “${folder.name}”${suffix}?`)) return;
  await runAction(root, state, async () => {
    const removed = new Set([folder.id, ...descendantIds(state, folder.id)]);
    await deleteConnectionFolder(folder.id);
    state.folders = state.folders.filter((item) => !removed.has(item.id));
    removed.forEach((id) => state.expandedIds.delete(id));
    state.selectedId = state.folders[0]?.id ?? null;
  });
}

function bindResizer(root, state) {
  const module = root.querySelector(".connections-module");
  const resizer = root.querySelector('[data-role="connections-resizer"]');
  const resizeTo = (width) => {
    const maximum = Math.max(240, module.getBoundingClientRect().width - 280);
    state.browserWidth = Math.round(Math.min(maximum, Math.max(240, width)));
    module.style.setProperty("--connections-browser-width", `${state.browserWidth}px`);
  };
  resizer.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    resizer.setPointerCapture(event.pointerId);
    module.classList.add("is-resizing");
  });
  resizer.addEventListener("pointermove", (event) => {
    if (!resizer.hasPointerCapture(event.pointerId)) return;
    resizeTo(event.clientX - module.getBoundingClientRect().left);
  });
  const finish = (event) => {
    if (!resizer.hasPointerCapture(event.pointerId)) return;
    resizer.releasePointerCapture(event.pointerId);
    module.classList.remove("is-resizing");
    localStorage.setItem(BROWSER_WIDTH_KEY, String(state.browserWidth));
  };
  resizer.addEventListener("pointerup", finish);
  resizer.addEventListener("pointercancel", finish);
  resizer.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    resizeTo(state.browserWidth + (event.key === "ArrowLeft" ? -20 : 20));
    localStorage.setItem(BROWSER_WIDTH_KEY, String(state.browserWidth));
  });
}

function showCreate(root, state, parentId) {
  state.createParentId = parentId;
  if (parentId) state.expandedIds.add(parentId);
  render(root, state);
  root.querySelector('[data-role="new-folder-name"]').focus();
}

async function runAction(root, state, action) {
  state.error = "";
  try {
    await action();
  } catch (cause) {
    state.error = String(cause);
  }
  render(root, state);
}

function folderById(state, id) {
  return state.folders.find((folder) => folder.id === id) ?? null;
}

function storedBrowserWidth() {
  const width = Number(localStorage.getItem(BROWSER_WIDTH_KEY));
  return Number.isFinite(width) && width >= 240 ? width : 410;
}

function childrenOf(state, parentId) {
  return state.folders
    .filter((folder) => (folder.parentId ?? null) === parentId)
    .sort((left, right) => left.name.localeCompare(right.name));
}

function connectionsInFolder(state, folderId) {
  return state.connections
    .filter((connection) => connection.folderId === folderId)
    .sort((left, right) => left.name.localeCompare(right.name));
}

function descendantIds(state, folderId) {
  return childrenOf(state, folderId).flatMap((folder) => [folder.id, ...descendantIds(state, folder.id)]);
}

function folderPath(state, id) {
  const parts = [];
  let folder = folderById(state, id);
  while (folder) {
    parts.unshift(folder.name);
    folder = folderById(state, folder.parentId);
  }
  return parts.join(" / ");
}

function authenticationOption(value, label, selected) {
  return `<option value="${value}"${value === selected ? " selected" : ""}>${label}</option>`;
}

function authenticationLabel(value) {
  return {
    password: "Password",
    "private-key": "Private key",
    "keyboard-interactive": "Keyboard interactive",
    "ssh-agent": "SSH agent",
  }[value] ?? "SSH";
}

async function copyText(value) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const field = document.createElement("textarea");
  field.value = value;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error("Could not copy hostname / IP.");
}

function icon(name) {
  const paths = {
    "folder-plus": "<path d='M3.5 7.5h6l2-2h9v13h-17zM15 10v5M12.5 12.5h5'/>",
    folder: "<path d='M3.5 7.5h6l2-2h9v13h-17z'/>",
    "folder-large": "<path d='M3.5 7.5h6l2-2h9v13h-17z'/>",
    chevron: "<path d='m9 7 5 5-5 5'/>",
    pencil: "<path d='m5 16-.7 3.7L8 19l10-10-3-3L5 16ZM13.5 7.5l3 3'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>",
    check: "<path d='m5 12 4 4L19 6'/>",
    close: "<path d='m7 7 10 10M17 7 7 17'/>",
    connection: "<path d='M8 12h8M12 8v8'/><circle cx='12' cy='12' r='8'/>",
    copy: "<rect x='8' y='8' width='11' height='11' rx='1.5'/><path d='M16 8V5H5v11h3'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}

function connectionIcon(type) {
  const paths = {
    ssh: "<path d='M4 5h16v14H4zM7 9l3 3-3 3M12 15h5'/>",
    rdp: "<rect x='3.5' y='5' width='17' height='12' rx='1.5'/><path d='M9 20h6M12 17v3'/>",
    url: "<path d='M8.5 8.5 6 11a3.5 3.5 0 0 0 5 5l2.5-2.5M15.5 15.5 18 13a3.5 3.5 0 0 0-5-5l-2.5 2.5M9 15l6-6'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[type]}</svg>`;
}
