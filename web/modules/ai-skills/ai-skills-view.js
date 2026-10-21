import { escapeHtml } from "../../shared/ui.js";
import {
  createAiSkillLibrary,
  deleteAiSkillLibrary,
  getAiSkills,
  readAiSkill,
  refreshAiSkills,
  runAiSkill,
  updateAiSkillLibrary,
} from "./ai-skills-api.js";

export async function mountAiSkillsView(root, detail = {}, { onCatalogChange = () => {} } = {}) {
  const state = {
    section: detail.section || "skills",
    libraries: [],
    skills: [],
    groups: [],
    selectedId: null,
    document: null,
    editingId: null,
    name: "",
    path: "",
    message: "",
    isError: false,
    busy: false,
    running: false,
    runOutput: "",
    runStatus: "",
    onCatalogChange,
    mountId: crypto.randomUUID(),
    runRequestId: null,
  };
  root.innerHTML = `<div class="ai-skills-loading" data-ai-skills-mount="${state.mountId}">Loading skills…</div>`;
  try {
    let snapshot = await getAiSkills();
    if (snapshot.libraries.length && !snapshot.skills.length) {
      try {
        await refreshAiSkills();
        snapshot = await getAiSkills();
      } catch (cause) {
        state.message = String(cause);
        state.isError = true;
      }
    }
    state.libraries = snapshot.libraries;
    state.skills = snapshot.skills;
    state.groups = snapshot.groups;
    state.selectedId = state.skills.some((skill) => skill.id === detail.skillId)
      ? detail.skillId
      : state.skills[0]?.id ?? null;
    if (state.selectedId) state.document = await readAiSkill(state.selectedId);
    if (!isSkillViewMounted(root, state)) return;
    state.onCatalogChange(state.libraries, state.skills, state.groups);
    render(root, state);
    if (detail.libraryId) {
      const row = root.querySelector(`[data-library-id="${CSS.escape(detail.libraryId)}"]`)?.closest(".skill-library-row");
      if (row) {
        row.classList.add("is-search-target");
        row.scrollIntoView({ block: "center" });
      }
    }
  } catch (cause) {
    if (isSkillViewMounted(root, state)) root.innerHTML = `<div class="ai-skills-error">${escapeHtml(cause)}</div>`;
  }
}

function isSkillViewMounted(root, state) {
  return root.isConnected && root.querySelector("[data-ai-skills-mount]")?.dataset.aiSkillsMount === state.mountId;
}

function appendSkillOutput(root, state, chunk) {
  const pre = root.querySelector(".skill-run-output");
  const follow = !pre || pre.scrollHeight - pre.clientHeight - pre.scrollTop < 24;
  if (!state.runOutput && pre) pre.textContent = "";
  state.runOutput += chunk;
  if (pre) {
    pre.append(document.createTextNode(chunk));
    if (follow) pre.scrollTop = pre.scrollHeight;
  }
}

function render(root, state) {
  const oldOutput = root.querySelector(".skill-run-output");
  const position = oldOutput ? { top: oldOutput.scrollTop, follow: oldOutput.scrollHeight - oldOutput.clientHeight - oldOutput.scrollTop < 24 } : null;
  root.innerHTML = `
    <div class="ai-skills-module" data-ai-skills-mount="${state.mountId}">
      ${state.section === "config" ? configView(state) : skillsView(state)}
      ${state.message ? `<p class="ai-skills-toast${state.isError ? " is-error" : ""}">${escapeHtml(state.message)}</p>` : ""}
    </div>`;
  bindEvents(root, state);
  const output = root.querySelector(".skill-run-output");
  if (output && position) output.scrollTop = position.follow ? output.scrollHeight : position.top;
}

function configView(state) {
  return `
    <section class="skill-config">
      <header class="skill-config-heading">
        <strong>Skill Config</strong>
        <span>Add and manage the local folders that contain your full skill packages.</span>
      </header>
      <form class="skill-library-form" data-role="library-form">
        <input data-role="library-name" value="${escapeHtml(state.name)}" aria-label="Folder name" placeholder="Folder name" autocomplete="off" />
        <input data-role="library-path" value="${escapeHtml(state.path)}" aria-label="Local skill folder" placeholder="Local skill folder" autocomplete="off" spellcheck="false" />
        <button type="button" data-action="choose-folder">Choose…</button>
        <button class="skill-primary-button" type="submit" ${state.name.trim() && state.path.trim() && !state.busy ? "" : "disabled"}>${state.editingId ? "Save" : "Add folder"}</button>
        ${state.editingId ? '<button type="button" data-action="cancel-edit">Cancel</button>' : ""}
      </form>
      <div class="skill-library-heading">
        <strong>Mounted skill folders</strong>
        <button data-action="refresh" ${state.libraries.length && !state.busy ? "" : "disabled"}>${icon("refresh")}<span>Refresh skills</span></button>
      </div>
      <div class="skill-library-table">
        <div class="skill-library-header"><span>Name</span><span>Path</span><span>Actions</span></div>
        ${state.libraries.length ? state.libraries.map((library) => `
          <div class="skill-library-row">
            <strong>${escapeHtml(library.name)}</strong>
            <code title="${escapeHtml(library.path)}">${escapeHtml(library.path)}</code>
            <span class="skill-library-actions">
              <button data-action="edit-library" data-library-id="${library.id}" title="Edit folder" aria-label="Edit ${escapeHtml(library.name)}">${icon("pencil")}</button>
              <button data-action="delete-library" data-library-id="${library.id}" title="Delete folder" aria-label="Delete ${escapeHtml(library.name)}">${icon("trash")}</button>
            </span>
          </div>`).join("") : '<p class="ai-skills-empty">No skill folders configured.</p>'}
      </div>
    </section>`;
}

function skillsView(state) {
  return `
    <section class="skills-catalog">
      <article class="skill-document">
        ${state.document ? `
          <header>
            <span>${icon("sparkles")}</span>
            <div><strong>${escapeHtml(state.document.name)}</strong><code>${escapeHtml(state.document.path)}</code></div>
            <div class="skill-document-actions">
              <button class="skill-run-button" data-action="run-skill" title="Run skill" aria-label="Run skill" ${state.running ? "disabled" : ""}>${icon(state.running ? "loader" : "play")}</button>
              <button data-action="refresh" title="Refresh skills" aria-label="Refresh skills" ${state.libraries.length && !state.busy && !state.running ? "" : "disabled"}>${icon("refresh")}</button>
            </div>
          </header>
          ${state.running ? '<div class="skill-running"><i></i><span>Running skill…</span></div>' : ""}
          ${state.running || state.runOutput ? `<div class="skill-output-heading"><strong>Output</strong><span>${escapeHtml(state.runStatus)}</span><button data-action="close-output" ${state.running ? "disabled" : ""}>Show SKILL.md</button></div><pre class="skill-run-output">${escapeHtml(state.runOutput || "Waiting for output…")}</pre>` : `<pre>${escapeHtml(state.document.content)}</pre>`}` : `
          <div class="skill-document-empty">${icon("sparkles")}<span>Select a skill</span></div>`}
      </article>
    </section>`;
}

function bindEvents(root, state) {
  root.querySelector('[data-role="library-form"]')?.addEventListener("submit", async (event) => {
    event.preventDefault();
    syncForm(root, state);
    await runAction(root, state, async () => {
      if (state.editingId) {
        await updateAiSkillLibrary(state.editingId, state.name, state.path);
      } else {
        await createAiSkillLibrary(state.name, state.path);
      }
      const result = await refreshAiSkills();
      const snapshot = await getAiSkills();
      state.libraries = snapshot.libraries;
      state.skills = snapshot.skills;
      state.groups = snapshot.groups;
      state.selectedId = state.skills[0]?.id ?? null;
      state.document = state.selectedId ? await readAiSkill(state.selectedId) : null;
      const skipped = result.skippedLibraries.length ? ` Skipped: ${result.skippedLibraries.join(", ")}.` : "";
      state.message = `Found ${result.discovered} skill${result.discovered === 1 ? "" : "s"}.${skipped}`;
      state.onCatalogChange(state.libraries, state.skills, state.groups);
      clearEditor(state, true);
    });
  });
  root.querySelector('[data-role="library-name"]')?.addEventListener("input", () => updateFormButton(root, state));
  root.querySelector('[data-role="library-path"]')?.addEventListener("input", () => updateFormButton(root, state));
  root.querySelector('[data-action="choose-folder"]')?.addEventListener("click", async () => {
    const selected = await window.__TAURI__.dialog.open({
      directory: true,
      multiple: false,
      canCreateDirectories: false,
      title: "Choose local skill folder",
    });
    if (!selected) return;
    state.path = selected;
    if (!state.name) state.name = selected.split(/[\\/]/).filter(Boolean).pop() ?? "";
    render(root, state);
  });
  root.querySelector('[data-action="cancel-edit"]')?.addEventListener("click", () => {
    clearEditor(state);
    render(root, state);
  });
  root.querySelectorAll('[data-action="edit-library"]').forEach((button) => {
    button.addEventListener("click", () => {
      const library = state.libraries.find((item) => item.id === button.dataset.libraryId);
      state.editingId = library.id;
      state.name = library.name;
      state.path = library.path;
      state.message = "";
      render(root, state);
      root.querySelector('[data-role="library-name"]')?.focus();
    });
  });
  root.querySelectorAll('[data-action="delete-library"]').forEach((button) => {
    button.addEventListener("click", async () => {
      const library = state.libraries.find((item) => item.id === button.dataset.libraryId);
      if (!window.confirm(`Delete skill folder “${library.name}” from GateTree-Ops?`)) return;
      await runAction(root, state, async () => {
        await deleteAiSkillLibrary(library.id);
        state.libraries = state.libraries.filter((item) => item.id !== library.id);
        state.skills = state.skills.filter((skill) => skill.libraryId !== library.id);
        state.groups = state.groups
          .map((group) => ({ ...group, skillIds: group.skillIds.filter((id) => state.skills.some((skill) => skill.id === id)) }))
          .filter((group) => group.skillIds.length);
        if (state.editingId === library.id) clearEditor(state);
        if (!state.skills.some((skill) => skill.id === state.selectedId)) {
          state.selectedId = state.skills[0]?.id ?? null;
          state.document = state.selectedId ? await readAiSkill(state.selectedId) : null;
        }
        state.message = "Skill folder removed.";
        state.onCatalogChange(state.libraries, state.skills, state.groups);
      });
    });
  });
  root.querySelector('[data-action="refresh"]')?.addEventListener("click", async () => {
    await runAction(root, state, async () => {
      const result = await refreshAiSkills();
      const snapshot = await getAiSkills();
      state.libraries = snapshot.libraries;
      state.skills = snapshot.skills;
      state.groups = snapshot.groups;
      if (!state.skills.some((skill) => skill.id === state.selectedId)) state.selectedId = state.skills[0]?.id ?? null;
      state.document = state.selectedId ? await readAiSkill(state.selectedId) : null;
      state.onCatalogChange(state.libraries, state.skills, state.groups);
      const skipped = result.skippedLibraries.length ? ` Skipped: ${result.skippedLibraries.join(", ")}.` : "";
      state.message = `Found ${result.discovered} skill${result.discovered === 1 ? "" : "s"}.${skipped}`;
    });
  });
  root.querySelector('[data-action="run-skill"]')?.addEventListener("click", async () => {
    const skill = state.document;
    const input = await requestSkillInput(skill);
    if (input === null || state.running || !isSkillViewMounted(root, state)) return;
    const requestId = crypto.randomUUID(); state.runRequestId = requestId;
    state.running = true;
    state.runOutput = "";
    state.runStatus = "Running";
    state.message = "";
    state.isError = false;
    render(root, state);
    try {
      const result = await runAiSkill(skill.id, input, chunk => {
        if (!state.running || state.runRequestId !== requestId || !isSkillViewMounted(root, state) || typeof chunk !== "string" || !chunk) return;
        appendSkillOutput(root, state, chunk);
      });
      state.runOutput = result.output;
      state.runStatus = result.status;
    } catch (cause) {
      const error = String(cause);
      state.runOutput = state.runOutput && !error.includes(state.runOutput)
        ? `${state.runOutput}\n\n${error}` : error;
      state.runStatus = "Failed";
      state.isError = true;
    } finally {
      state.running = false;
      if (state.runRequestId === requestId && isSkillViewMounted(root, state)) render(root, state);
    }
  });
  root.querySelector('[data-action="close-output"]')?.addEventListener("click", () => {
    state.runOutput = "";
    state.runStatus = "";
    render(root, state);
  });
}

function requestSkillInput(document) {
  const dialog = window.document.createElement("dialog");
  dialog.className = "ssh-auth-dialog skill-run-dialog";
  dialog.innerHTML = `
    <form>
      <strong>Run ${escapeHtml(document.name)}</strong>
      <small>${escapeHtml(document.path)}</small>
      <label><span>${escapeHtml(document.inputPrompt || "Task (optional)")}</span><textarea data-role="skill-input" spellcheck="false"></textarea></label>
      <div><button type="button" data-action="cancel-run">Cancel</button><button class="connection-primary-action" type="submit">Run</button></div>
    </form>`;
  window.document.body.append(dialog);
  return new Promise((resolve) => {
    const close = (result) => {
      dialog.close();
      dialog.remove();
      resolve(result);
    };
    dialog.querySelector('[data-action="cancel-run"]').addEventListener("click", () => close(null));
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      close(null);
    });
    dialog.querySelector("form").addEventListener("submit", (event) => {
      event.preventDefault();
      close(dialog.querySelector('[data-role="skill-input"]').value.trim());
    });
    dialog.showModal();
    dialog.querySelector("textarea").focus();
  });
}

function syncForm(root, state) {
  state.name = root.querySelector('[data-role="library-name"]')?.value ?? state.name;
  state.path = root.querySelector('[data-role="library-path"]')?.value ?? state.path;
}

function updateFormButton(root, state) {
  syncForm(root, state);
  root.querySelector('.skill-library-form [type="submit"]').disabled = !state.name.trim() || !state.path.trim();
}

async function runAction(root, state, action) {
  state.busy = true;
  state.message = "";
  state.isError = false;
  try {
    await action();
  } catch (cause) {
    state.message = String(cause);
    state.isError = true;
  } finally {
    state.busy = false;
    render(root, state);
  }
}

function clearEditor(state, keepMessage = false) {
  state.editingId = null;
  state.name = "";
  state.path = "";
  if (!keepMessage) state.message = "";
}

function icon(name) {
  const paths = {
    refresh: "<path d='M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6'/>",
    pencil: "<path d='m5 16-.7 3.7L8 19l10-10-3-3L5 16ZM13.5 7.5l3 3'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>",
    sparkles: "<path d='m12 3 1.2 4.1L17 8.5l-3.8 1.4L12 14l-1.2-4.1L7 8.5l3.8-1.4L12 3ZM18 14l.7 2.3L21 17l-2.3.7L18 20l-.7-2.3L15 17l2.3-.7L18 14Z'/>",
    play: "<path d='m8 5 11 7-11 7z'/>",
    loader: "<path d='M12 3a9 9 0 1 1-9 9'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
