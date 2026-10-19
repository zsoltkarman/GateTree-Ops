import { escapeHtml } from "../../shared/ui.js";
import { createModuleRoot } from "../../shared/module-root.js";
import { focusNavigationItem } from "../../shared/focus-navigation.js";
import {
  createNote,
  createNoteFolder,
  deleteNote,
  deleteNoteFolder,
  listNotes,
  moveNote,
  renameNoteFolder,
  updateNote,
} from "./notes-api.js";

export async function mountNotesView(contentRoot, options = {}) {
  const root = createModuleRoot(contentRoot, options.navigationRoot);
  const state = {
    folders: [],
    notes: [],
    folderId: null,
    noteId: null,
    draftTitle: "",
    draftBody: "",
    dirty: false,
    error: "",
    expandedFolderIds: new Set(),
    navigationRoot: options.navigationRoot ?? null,
  };

  root.innerHTML = '<div class="notes-loading">Loading encrypted notes…</div>';
  try {
    const snapshot = await listNotes();
    state.folders = snapshot.folders;
    state.notes = snapshot.notes;
    const searchedNote = state.notes.find((note) => note.id === options.noteId);
    state.folderId = searchedNote ? searchedNote.folderId : state.folders.find((folder) => folder.id === options.folderId)?.id ?? (state.notes.some((note) => !note.folderId) ? null : state.folders[0]?.id ?? null);
    state.noteId = searchedNote?.id ?? notesInFolder(state)[0]?.id ?? null;
    if ((searchedNote || options.folderId) && state.folderId) state.expandedFolderIds.add(state.folderId);
    loadDraft(state);
    render(root, state);
    if (searchedNote) {
      focusNavigationItem(root, `.note-row[data-note-id="${CSS.escape(searchedNote.id)}"]`);
    } else if (options.folderId && state.folderId) {
      focusNavigationItem(root, `.folder-main[data-folder-id="${CSS.escape(state.folderId)}"]`);
    }
  } catch (cause) {
    root.innerHTML = `<div class="notes-error">${escapeHtml(cause)}</div>`;
  }
}

function render(root, state) {
  const selected = selectedNote(state);
  root.innerHTML = `
    <div class="notes-module${state.navigationRoot ? " is-main-navigation-layout" : ""}">
      <aside class="notes-browser">
        <div class="note-folders">
          ${state.notes.filter((note) => !note.folderId).map((note) => noteRow(note, state)).join("")}
          ${state.folders.map((folder) => folderTree(folder, state)).join("")}
          ${!state.folders.length && !state.notes.length ? '<p class="notes-empty-list">Right-click to create a note or folder</p>' : ""}
        </div>
      </aside>
      <section class="note-editor">
        ${selected ? `
          <div class="note-editor-toolbar">
            <input class="note-title" data-role="note-title" value="${escapeHtml(state.draftTitle)}" placeholder="Untitled note" />
            <select data-role="note-folder" aria-label="Note folder">
              <option value=""${!selected.folderId ? " selected" : ""}>Notes</option>
              ${state.folders.map((folder) => `<option value="${folder.id}"${selected.folderId === folder.id ? " selected" : ""}>${escapeHtml(folder.name)}</option>`).join("")}
            </select>
            <span class="unsaved-indicator" data-role="unsaved" ${state.dirty ? "" : "hidden"}>Unsaved</span>
            <button class="note-save" data-action="save-note" title="Save note" aria-label="Save note" ${state.dirty ? "" : "disabled"}>${icon("save")}</button>
            <button class="note-delete" data-action="delete-note" title="Delete note" aria-label="Delete note">${icon("trash")}</button>
          </div>
          <textarea class="plain-note-editor" data-role="note-body" spellcheck="false" autocapitalize="off" autocomplete="off" wrap="soft">${escapeHtml(state.draftBody)}</textarea>
        ` : `
          <div class="notes-empty-editor">
            ${icon("note")}
            <strong>No note selected</strong>
            <span>Create a note with the + button.</span>
          </div>`}
      </section>
      <div class="notes-toast${state.error ? " is-error" : ""}" data-role="notes-toast" ${state.error ? "" : "hidden"}>${escapeHtml(state.error)}</div>
    </div>`;

  bindEvents(root, state);
  if (state.navigationRoot) root.mountNavigation(".notes-browser");
}

function bindEvents(root, state) {
  bindNotesContextMenu(root, state);
  root.querySelectorAll('[data-action="select-folder"]').forEach((row) => {
    row.addEventListener("click", () => {
      if (!confirmDiscard(state)) return;
      state.folderId = row.dataset.folderId;
      state.expandedFolderIds.add(state.folderId);
      state.noteId = notesInFolder(state)[0]?.id ?? null;
      loadDraft(state);
      render(root, state);
    });
  });
  root.querySelectorAll('[data-action="toggle-folder"]').forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.folderId;
      if (state.expandedFolderIds.has(id)) state.expandedFolderIds.delete(id);
      else state.expandedFolderIds.add(id);
      render(root, state);
    });
  });
  root.querySelectorAll("[data-note-id]").forEach((row) => {
    row.addEventListener("click", () => {
      if (!confirmDiscard(state)) return;
      state.noteId = row.dataset.noteId;
      state.folderId = selectedNote(state).folderId;
      loadDraft(state);
      render(root, state);
    });
  });

  root.querySelectorAll('[data-action="rename-folder"]').forEach((button) => {
    button.addEventListener("click", async () => {
      const folder = state.folders.find((item) => item.id === button.dataset.folderId);
      const name = window.prompt("Folder name", folder.name);
      if (name === null || !name.trim() || name.trim() === folder.name) return;
      await runAction(root, state, async () => {
        const updated = await renameNoteFolder(folder.id, name);
        Object.assign(folder, updated);
        state.folders.sort((left, right) => left.name.localeCompare(right.name));
      });
    });
  });
  root.querySelectorAll('[data-action="delete-folder"]').forEach((button) => {
    button.addEventListener("click", async () => {
      const folder = state.folders.find((item) => item.id === button.dataset.folderId);
      if (!window.confirm(`Delete the empty folder “${folder.name}”?`)) return;
      if (!confirmDiscard(state)) return;
      await runAction(root, state, async () => {
        await deleteNoteFolder(folder.id);
        state.folders = state.folders.filter((item) => item.id !== folder.id);
        state.expandedFolderIds.delete(folder.id);
        if (state.folderId === folder.id) {
          state.folderId = state.folders[0]?.id ?? null;
          state.noteId = notesInFolder(state)[0]?.id ?? null;
          loadDraft(state);
        }
      });
    });
  });

  const title = root.querySelector('[data-role="note-title"]');
  const body = root.querySelector('[data-role="note-body"]');
  const markDirty = () => {
    state.draftTitle = title.value;
    state.draftBody = body.value;
    state.dirty = true;
    root.querySelector('[data-role="unsaved"]').hidden = false;
    root.querySelector('[data-action="save-note"]').disabled = false;
  };
  title?.addEventListener("input", markDirty);
  body?.addEventListener("input", markDirty);
  body?.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      root.querySelector('[data-action="save-note"]').click();
    }
  });
  root.querySelector('[data-action="save-note"]')?.addEventListener("click", async () => {
    await runAction(root, state, async () => {
      const updated = await updateNote(state.noteId, state.draftTitle, state.draftBody);
      replaceNote(state, updated);
      state.dirty = false;
    });
  });
  root.querySelector('[data-role="note-folder"]')?.addEventListener("change", async (event) => {
    if (state.dirty && !window.confirm("Move this note without saving the current changes?")) {
      event.target.value = selectedNote(state).folderId ?? "";
      return;
    }
    const folderId = event.target.value || null;
    await runAction(root, state, async () => {
      const updated = await moveNote(state.noteId, folderId);
      replaceNote(state, updated);
      state.folderId = folderId;
      state.noteId = updated.id;
      if (folderId) state.expandedFolderIds.add(folderId);
      loadDraft(state);
    });
  });
  root.querySelector('[data-action="delete-note"]')?.addEventListener("click", async () => {
    if (!window.confirm("Delete this note?")) return;
    await runAction(root, state, async () => {
      await deleteNote(state.noteId);
      state.notes = state.notes.filter((note) => note.id !== state.noteId);
      state.noteId = notesInFolder(state)[0]?.id ?? null;
      loadDraft(state);
    });
  });
}

function bindNotesContextMenu(root, state) {
  const open = (event) => {
    event.preventDefault();
    event.stopPropagation();
    const folderRow = event.target.closest(".folder-row");
    const noteRow = event.target.closest("[data-note-id]");
    const folderId = folderRow ? folderRow.querySelector("[data-folder-id]")?.dataset.folderId
      : noteRow ? state.notes.find((note) => note.id === noteRow.dataset.noteId)?.folderId ?? null
      : event.currentTarget.classList.contains("notes-browser") ? null : state.folderId;
    showNotesContextMenu(root, state, event.clientX, event.clientY, folderId);
  };
  root.querySelector(".notes-browser")?.addEventListener("contextmenu", open);
  root.querySelector(".notes-module")?.addEventListener("contextmenu", open);
}

function showNotesContextMenu(root, state, x, y, targetFolderId) {
  document.querySelector(".notes-context-menu")?.remove();
  const menu = document.createElement("div");
  menu.className = "connection-context-menu notes-context-menu";
  menu.innerHTML = `
    <button data-notes-context="folder">${icon("folder-plus")}<span>New folder</span></button>
    <button data-notes-context="note">${icon("note")}<span>New note</span></button>`;
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

  menu.querySelector('[data-notes-context="folder"]').addEventListener("click", async () => {
    close();
    const folder = await createNoteFolderWithDialog();
    if (!folder) return;
    state.folders.push(folder);
    state.folders.sort((left, right) => left.name.localeCompare(right.name));
    state.folderId = folder.id;
    state.noteId = null;
    state.expandedFolderIds.add(folder.id);
    loadDraft(state);
    render(root, state);
  });
  menu.querySelector('[data-notes-context="note"]').addEventListener("click", async () => {
    close();
    if (!confirmDiscard(state)) return;
    await runAction(root, state, async () => {
      state.folderId = targetFolderId;
      const note = await createNote(targetFolderId);
      state.notes.unshift(note);
      state.noteId = note.id;
      if (state.folderId) state.expandedFolderIds.add(state.folderId);
      loadDraft(state);
    });
  });
}

function createNoteFolderWithDialog() {
  const dialog = document.createElement("dialog");
  dialog.className = "ssh-auth-dialog notes-folder-dialog";
  dialog.innerHTML = `
    <form>
      <strong>New folder</strong>
      <label><span>Name</span><input data-role="notes-folder-name" maxlength="120" required autocomplete="off" /></label>
      <p class="ssh-auth-error" data-role="notes-folder-error" hidden></p>
      <div><button type="button" data-action="cancel-folder">Cancel</button><button class="connection-primary-action" type="submit">Add folder</button></div>
    </form>`;
  document.body.append(dialog);
  return new Promise((resolve) => {
    const close = (result) => {
      dialog.close();
      dialog.remove();
      resolve(result);
    };
    dialog.querySelector('[data-action="cancel-folder"]').addEventListener("click", () => close(null));
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      close(null);
    });
    dialog.querySelector("form").addEventListener("submit", async (event) => {
      event.preventDefault();
      const nameInput = dialog.querySelector('[data-role="notes-folder-name"]');
      const submit = dialog.querySelector('[type="submit"]');
      const error = dialog.querySelector('[data-role="notes-folder-error"]');
      const name = nameInput.value.trim();
      if (!name) return;
      submit.disabled = true;
      error.hidden = true;
      try {
        close(await createNoteFolder(name));
      } catch (cause) {
        error.textContent = String(cause);
        error.hidden = false;
        submit.disabled = false;
        nameInput.select();
      }
    });
    dialog.showModal();
    dialog.querySelector("input").focus();
  });
}

async function runAction(root, state, action) {
  try {
    state.error = "";
    await action();
  } catch (cause) {
    state.error = String(cause);
  }
  render(root, state);
}

function folderTree(folder, state) {
  const folderNotes = state.notes.filter((note) => note.folderId === folder.id);
  const active = state.folderId === folder.id;
  const expanded = state.expandedFolderIds.has(folder.id);
  return `
    <div class="folder-tree-item">
      <div class="folder-row${active ? " is-active" : ""}">
        <button class="folder-toggle${folderNotes.length ? "" : " is-empty"}" data-action="toggle-folder" data-folder-id="${folder.id}" aria-label="${expanded ? "Collapse" : "Expand"} ${escapeHtml(folder.name)}" ${folderNotes.length ? "" : "disabled"}>
          ${icon("chevron")}
        </button>
        <button class="folder-main" data-action="select-folder" data-folder-id="${folder.id}">
          ${icon("folder")}
          <span>${escapeHtml(folder.name)}</span>
          <b>${folderNotes.length}</b>
        </button>
        <span class="folder-row-actions">
          <button data-action="rename-folder" data-folder-id="${folder.id}" title="Rename folder" aria-label="Rename ${escapeHtml(folder.name)}">${icon("pencil")}</button>
          <button data-action="delete-folder" data-folder-id="${folder.id}" title="${folderNotes.length ? "Delete or move the notes before deleting this folder" : "Delete folder"}" aria-label="Delete ${escapeHtml(folder.name)}" ${folderNotes.length ? "disabled" : ""}>${icon("trash")}</button>
        </span>
      </div>
      <div class="folder-notes" ${expanded && folderNotes.length ? "" : "hidden"}>
        ${folderNotes.map((note) => noteRow(note, state)).join("")}
      </div>
    </div>`;
}

function noteRow(note, state) {
  return `<button class="note-row${note.id === state.noteId ? " is-active" : ""}" data-note-id="${note.id}">
    ${icon("note")}<span><strong>${escapeHtml(note.title || "Untitled note")}</strong></span>
  </button>`;
}

function notesInFolder(state) {
  return state.notes.filter((note) => note.folderId === state.folderId);
}

function selectedNote(state) {
  return state.notes.find((note) => note.id === state.noteId) ?? null;
}

function loadDraft(state) {
  const note = selectedNote(state);
  state.draftTitle = note?.title ?? "";
  state.draftBody = note?.body ?? "";
  state.dirty = false;
}

function replaceNote(state, updated) {
  const index = state.notes.findIndex((note) => note.id === updated.id);
  if (index >= 0) state.notes[index] = updated;
}

function confirmDiscard(state) {
  return !state.dirty || window.confirm("Discard the unsaved changes to this note?");
}

function icon(name) {
  const paths = {
    plus: "<path d='M12 5v14M5 12h14'/>",
    "folder-plus": "<path d='M3.5 7.5h6l2-2h9v13h-17zM15 10v5M12.5 12.5h5'/>",
    folder: "<path d='M3.5 7.5h6l2-2h9v13h-17z'/>",
    chevron: "<path d='m9 7 5 5-5 5'/>",
    pencil: "<path d='m5 16-.7 3.7L8 19l10-10-3-3L5 16ZM13.5 7.5l3 3'/>",
    save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>",
    note: "<path d='M6 3.5h9l3 3V20H6zM14.5 3.5V7H18M9 11h6M9 14.5h6'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
