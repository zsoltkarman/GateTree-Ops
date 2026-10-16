import { bindThemePicker, themePickerMarkup } from "../appearance/theme-picker.js";

const SIDEBAR_STATE_KEY = "pharos.shell.sidebarCollapsed";
const SIDEBAR_WIDTH_KEY = "pharos.shell.sidebarWidth";
const SIDEBAR_MIN_WIDTH = 150;
const SIDEBAR_MAX_WIDTH = 420;

const navigation = [
  ["connections", "Connections", "<path d='M8.5 8.5 6 11a3.5 3.5 0 0 0 5 5l2.5-2.5M15.5 15.5 18 13a3.5 3.5 0 0 0-5-5l-2.5 2.5M9 15l6-6' />"],
  ["deployments", "Deployments", "<path d='M5 6.5h14v4H5zM5 13.5h14v4H5zM8 8.5h.01M8 15.5h.01' />"],
  ["ai-skills", "AI Skills", "<path d='m12 3 1.2 4.1L17 8.5l-3.8 1.4L12 14l-1.2-4.1L7 8.5l3.8-1.4L12 3ZM18 14l.7 2.3L21 17l-2.3.7L18 20l-.7-2.3L15 17l2.3-.7L18 14Z' />"],
  ["notes", "Notes", "<path d='M6 3.5h9l3 3V20H6zM14.5 3.5V7H18M9 11h6M9 14.5h6' />"],
  ["vaults", "Vaults", "<rect x='5' y='8.5' width='14' height='11' rx='2' /><path d='M8.5 8.5V6a3.5 3.5 0 0 1 7 0v2.5M12 13v2' />"],
  ["agent", "Agent", "<rect x='5' y='6.5' width='14' height='12' rx='3' /><path d='M9 11h.01M15 11h.01M9 15h6M12 3v3.5' />"],
  ["backup", "Backup", "<path d='M5 5h14v14H5zM8 5v5h8V5M8 19v-6h8v6M12 2v3M9.5 3.5 12 1l2.5 2.5' />"],
  ["help", "Help", "<circle cx='12' cy='12' r='8.5' /><path d='M9.8 9.5a2.3 2.3 0 1 1 3.2 2.1c-.7.3-1 1-1 1.7M12 16.7h.01' />"],
];

function icon(path) {
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${path}</svg>`;
}

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;",
  })[character]);
}

function navigationMarkup(id, label, iconPath, index) {
  const hasModuleNavigation = ["connections", "deployments", "ai-skills", "notes"].includes(id);
  const item = `
    <button class="navigation-item${index === 0 ? " is-active" : ""}" data-module="${id}" title="${label}">
      ${icon(iconPath)}
      <span>${label}</span>
      ${hasModuleNavigation ? icon("<path d='m9 7 5 5-5 5'/>") : ""}
    </button>`;
  if (!hasModuleNavigation) return item;
  return `
    <div class="navigation-group" data-navigation-group="${id}">
      ${item}
      <div class="navigation-children" data-role="${id}-navigation" hidden>
        ${id === "ai-skills" ? `
          <button data-ai-section="skills">${icon("<path d='m9 7 5 5-5 5'/>")}<span>Skills</span></button>
          <div data-role="ai-skill-links"></div>
          <button data-ai-section="config">${icon("<path d='M3.5 7.5h6l2-2h9v13h-17zM15 10v5M12.5 12.5h5'/>")}<span>Config</span></button>` : id === "deployments" ? `
          <button data-deployment-section="queue">${icon("<path d='M5 6h14M5 12h14M5 18h14M8 4v4M8 10v4M8 16v4'/>")}<span>Queue</span>${icon("<path d='m9 7 5 5-5 5'/>")}</button>
          <div data-role="deployment-queue-links"></div>
          <button data-deployment-section="config">${icon("<path d='M5 5h4v4H5zM15 5h4v4h-4zM10 7h5M17 9v5M15 14h4v4h-4zM9 16h6M5 14h4v4H5zM7 9v5'/>")}<span>Config</span>${icon("<path d='m9 7 5 5-5 5'/>")}</button>
          <div data-role="deployment-process-links"></div>` : `<div class="module-navigation-host" data-role="${id}-navigation-host"></div>`}
      </div>
    </div>`;
}

export function mountAppShell(root, { onLock, onBiometricToggle, onNavigate }) {
  root.innerHTML = `
    <main class="shell">
      <header class="shell-header">
        <button class="shell-menu-toggle" data-action="toggle-sidebar" aria-label="Hide menu" title="Hide menu">
          ${icon("<path d='M4 7h16M4 12h16M4 17h16' />")}
        </button>
        <label class="global-search">
          ${icon("<circle cx='10.5' cy='10.5' r='5.5' /><path d='m15 15 4 4' />")}
          <input type="search" placeholder="Search" aria-label="Search" autocomplete="off" />
        </label>
        <p class="shell-message" data-role="shell-message" role="status" hidden></p>
        <div class="shell-actions">
          ${themePickerMarkup()}
          <button class="icon-button biometric-button" data-action="biometric" aria-label="Enable biometric unlock" title="Enable biometric unlock" hidden>
            ${icon("<path d='M7.4 9.2a4.9 4.9 0 0 1 9.2 2.3M5.2 12a6.8 6.8 0 0 1 13.5-.2M8.7 12.1c0-4.2 6.5-4.2 6.5 0 0 3.8-1 6.6-2.5 8.3M5.8 15.1c.1 2.2-.4 3.7-1.2 5M11.9 11.4c0 4.2-.7 6.8-2.2 9.1M18.2 14.5c-.1 2.5-.6 4.5-1.5 6' />")}
          </button>
          <button class="icon-button" data-action="lock" aria-label="Log out" title="Log out">
            ${icon("<path d='M10 5H5.8A1.8 1.8 0 0 0 4 6.8v10.4A1.8 1.8 0 0 0 5.8 19H10M14.5 8.5 18 12l-3.5 3.5M9 12h9' />")}
          </button>
        </div>
      </header>
      <div class="shell-body">
        <aside class="sidebar" aria-label="Main navigation">
          <nav class="main-navigation">
            ${navigation.map(([id, label, iconPath], index) => navigationMarkup(id, label, iconPath, index)).join("")}
          </nav>
        </aside>
        <div class="sidebar-resizer" data-role="sidebar-resizer" role="separator" aria-label="Resize menu" aria-orientation="vertical" tabindex="0"></div>
        <section class="work-area" aria-label="Workspace content">
          <div class="module-content" data-role="module-content"></div>
        </section>
      </div>
    </main>`;

  const shell = root.querySelector(".shell");
  const sidebarToggle = root.querySelector('[data-action="toggle-sidebar"]');
  const biometricButton = root.querySelector('[data-action="biometric"]');
  const message = root.querySelector('[data-role="shell-message"]');
  const sidebarResizer = root.querySelector('[data-role="sidebar-resizer"]');
  let messageTimer = null;

  const storedWidth = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
  const initialWidth = Number.isFinite(storedWidth)
    ? Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, storedWidth))
    : 230;
  shell.style.setProperty("--sidebar-width", `${initialWidth}px`);

  const setSidebarWidth = (width) => {
    const bounded = Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(width)));
    shell.style.setProperty("--sidebar-width", `${bounded}px`);
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(bounded));
  };

  const setSidebarCollapsed = (collapsed) => {
    shell.classList.toggle("is-sidebar-collapsed", collapsed);
    sidebarToggle.title = collapsed ? "Show menu" : "Hide menu";
    sidebarToggle.setAttribute("aria-label", sidebarToggle.title);
    localStorage.setItem(SIDEBAR_STATE_KEY, String(collapsed));
  };
  setSidebarCollapsed(localStorage.getItem(SIDEBAR_STATE_KEY) === "true");
  bindThemePicker(root);

  const aiChildren = root.querySelector('[data-role="ai-skills-navigation"]');
  const aiLinks = root.querySelector('[data-role="ai-skill-links"]');
  let aiCatalog = { libraries: [], skills: [], groups: [] };
  let aiSkillsExpanded = true;
  const expandedAiGroups = new Set();
  const knownAiGroups = new Set();
  let deploymentCatalog = { plans: [], processes: [], section: "queue", selectedId: null, selectedPlanId: null };
  let deploymentQueueExpanded = true;
  let deploymentConfigExpanded = true;

  const setModuleExpanded = (module, expanded) => {
    const group = root.querySelector(`[data-navigation-group="${module}"]`);
    const children = root.querySelector(`[data-role="${module}-navigation"]`);
    if (!group || !children) return;
    group.classList.toggle("is-expanded", expanded);
    children.hidden = !expanded;
  };

  const selectAiChild = (detail = {}) => {
    aiChildren.querySelectorAll("button.is-active").forEach((button) => button.classList.remove("is-active"));
    const selector = detail.skillId
      ? `[data-ai-skill-id="${CSS.escape(detail.skillId)}"]`
      : `[data-ai-section="${detail.section || "skills"}"]`;
    aiChildren.querySelector(selector)?.classList.add("is-active");
  };

  const bindAiChildren = () => {
    aiChildren.querySelectorAll("[data-ai-section]").forEach((button) => {
      button.onclick = () => {
        if (button.dataset.aiSection === "skills") {
          aiSkillsExpanded = !aiSkillsExpanded;
          aiLinks.hidden = !aiSkillsExpanded;
          button.classList.toggle("is-expanded", aiSkillsExpanded);
        }
        navigate("ai-skills", { section: button.dataset.aiSection });
      };
    });
    aiChildren.querySelectorAll("[data-ai-group-toggle]").forEach((button) => {
      button.onclick = () => {
        const id = button.dataset.aiGroupToggle;
        if (expandedAiGroups.has(id)) expandedAiGroups.delete(id);
        else expandedAiGroups.add(id);
        renderAiCatalog();
      };
    });
    aiChildren.querySelectorAll("[data-ai-skill-id]").forEach((button) => {
      button.onclick = () => navigate("ai-skills", { section: "skills", skillId: button.dataset.aiSkillId });
    });
  };

  const renderAiCatalog = () => {
    aiCatalog.groups.forEach((group) => {
      if (!knownAiGroups.has(group.id)) {
        knownAiGroups.add(group.id);
        expandedAiGroups.add(group.id);
      }
    });
    aiLinks.hidden = !aiSkillsExpanded;
    aiLinks.innerHTML = aiCatalog.groups.map((group) => {
      const expanded = expandedAiGroups.has(group.id);
      const skills = group.skillIds
        .map((id) => aiCatalog.skills.find((skill) => skill.id === id))
        .filter(Boolean);
      const groupIcon = group.icon === "folder" ? "📁" : group.icon;
      return `<div class="ai-navigation-group">
        <button class="ai-navigation-group-toggle${expanded ? " is-expanded" : ""}" data-ai-group-toggle="${escapeHtml(group.id)}">
          ${icon("<path d='m9 7 5 5-5 5'/>")}<span>${escapeHtml(groupIcon)}&nbsp; ${escapeHtml(group.label)}</span>
        </button>
        <div class="ai-navigation-skills" ${expanded ? "" : "hidden"}>
          ${skills.map((skill) => `
            <button data-ai-skill-id="${escapeHtml(skill.id)}" title="${escapeHtml(skill.name)}">
              ${icon("<path d='m12 4 1 3.5 3 1-3 1L12 13l-1-3.5-3-1 3-1L12 4Z'/>")}
              <span>${escapeHtml(skill.name)}</span>
            </button>`).join("")}
        </div>
      </div>`;
    }).join("");
    const skillsButton = aiChildren.querySelector('[data-ai-section="skills"]');
    skillsButton?.classList.toggle("is-expanded", aiSkillsExpanded);
    bindAiChildren();
  };

  const renderDeploymentCatalog = () => {
    const renderLinks = (host, section, entries) => {
      host.innerHTML = entries.map((entry) => `
        <button class="deployment-navigation-resource${deploymentCatalog.section === section && deploymentCatalog.selectedId === entry.id ? " is-active" : ""}"
                data-deployment-resource="${escapeHtml(entry.id)}" data-deployment-resource-section="${section}" title="${escapeHtml(entry.name)}">
          ${icon("<path d='M7 6.5h10v11H7zM9.5 9.5h5M9.5 12h5M9.5 14.5h3'/>")}<span>${escapeHtml(entry.name)}</span>
        </button>`).join("");
    };
    const queueButton = root.querySelector('[data-deployment-section="queue"]');
    const queueLinks = root.querySelector('[data-role="deployment-queue-links"]');
    const hasPlans = deploymentCatalog.plans.length > 0;
    queueButton.classList.toggle("has-children", hasPlans);
    queueButton.classList.toggle("is-expanded", hasPlans && deploymentQueueExpanded);
    queueButton.querySelector("svg:last-child").hidden = !hasPlans;
    queueLinks.hidden = !hasPlans || !deploymentQueueExpanded;
    queueLinks.innerHTML = deploymentCatalog.plans.map((plan) => `
      <button class="deployment-navigation-resource${deploymentCatalog.section === "queue" && deploymentCatalog.selectedPlanId === plan.id ? " is-active" : ""}"
              data-queue-plan="${escapeHtml(plan.id)}" title="${escapeHtml(plan.name)}">
        ${icon("<path d='M6 4h12v16H6zM9 8h6M9 12h6M9 16h4'/>")}<span>${escapeHtml(plan.name)}</span>
      </button>`).join("");
    queueLinks.querySelectorAll("[data-queue-plan]").forEach((button) => {
      button.addEventListener("click", () => navigate("deployments", { section: "queue", planId: button.dataset.queuePlan }));
    });
    const configButton = root.querySelector('[data-deployment-section="config"]');
    const processLinks = root.querySelector('[data-role="deployment-process-links"]');
    const hasConfigs = deploymentCatalog.processes.length > 0;
    configButton.classList.toggle("has-children", hasConfigs);
    configButton.classList.toggle("is-expanded", hasConfigs && deploymentConfigExpanded);
    configButton.querySelector("svg:last-child").hidden = !hasConfigs;
    processLinks.hidden = !hasConfigs || !deploymentConfigExpanded;
    renderLinks(processLinks, "config", deploymentCatalog.processes);
    root.querySelectorAll("[data-deployment-resource]").forEach((button) => {
      button.addEventListener("click", () => navigate("deployments", {
        section: "config",
        resourceId: button.dataset.deploymentResource,
      }));
      button.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        event.stopPropagation();
        document.querySelector(".deployment-context-menu")?.remove();
        const menu = document.createElement("div");
        menu.className = "connection-context-menu deployment-context-menu";
        menu.innerHTML = `<button>${icon("<path d='M4 20h4l11-11-4-4L4 16zM13.5 6.5l4 4'/>")}<span>Edit Config</span></button>`;
        menu.style.left = `${Math.min(event.clientX, window.innerWidth - 190)}px`;
        menu.style.top = `${Math.min(event.clientY, window.innerHeight - 48)}px`;
        document.body.append(menu);
        menu.querySelector("button").addEventListener("click", () => {
          menu.remove();
          navigate("deployments", { section: "config", resourceId: button.dataset.deploymentResource });
        });
        window.setTimeout(() => document.addEventListener("pointerdown", () => menu.remove(), { once: true }));
      });
    });
  };

  const openDeploymentContextMenu = (event, section) => {
    event.preventDefault();
    event.stopPropagation();
    document.querySelector(".deployment-context-menu")?.remove();
    const menu = document.createElement("div");
    menu.className = "connection-context-menu deployment-context-menu";
    const label = section === "queue" ? "New Plan" : "New Config";
    menu.innerHTML = `<button>${icon("<path d='M12 5v14M5 12h14'/>")}<span>${label}</span></button>`;
    menu.style.left = `${Math.min(event.clientX, window.innerWidth - 190)}px`;
    menu.style.top = `${Math.min(event.clientY, window.innerHeight - 48)}px`;
    document.body.append(menu);
    menu.querySelector("button").addEventListener("click", () => {
      menu.remove();
      navigate("deployments", { section, new: true });
    });
    window.setTimeout(() => {
      document.addEventListener("pointerdown", () => menu.remove(), { once: true });
    });
  };

  sidebarToggle.addEventListener("click", () => {
    setSidebarCollapsed(!shell.classList.contains("is-sidebar-collapsed"));
  });
  sidebarResizer.addEventListener("pointerdown", (event) => {
    if (shell.classList.contains("is-sidebar-collapsed")) return;
    event.preventDefault();
    sidebarResizer.setPointerCapture(event.pointerId);
    shell.classList.add("is-sidebar-resizing");
    const startX = event.clientX;
    const startWidth = Number.parseFloat(getComputedStyle(shell).getPropertyValue("--sidebar-width"));
    const move = (moveEvent) => setSidebarWidth(startWidth + moveEvent.clientX - startX);
    const stop = () => {
      shell.classList.remove("is-sidebar-resizing");
      sidebarResizer.removeEventListener("pointermove", move);
      sidebarResizer.removeEventListener("pointerup", stop);
      sidebarResizer.removeEventListener("pointercancel", stop);
    };
    sidebarResizer.addEventListener("pointermove", move);
    sidebarResizer.addEventListener("pointerup", stop);
    sidebarResizer.addEventListener("pointercancel", stop);
  });
  sidebarResizer.addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight"].includes(event.key) || shell.classList.contains("is-sidebar-collapsed")) return;
    event.preventDefault();
    const width = Number.parseFloat(getComputedStyle(shell).getPropertyValue("--sidebar-width"));
    setSidebarWidth(width + (event.key === "ArrowRight" ? 12 : -12));
  });
  const navigate = (module, detail = null) => {
    const item = root.querySelector(`.navigation-item[data-module="${module}"]`);
    if (!item) return;
    root.querySelector(".navigation-item.is-active")?.classList.remove("is-active");
    item.classList.add("is-active");
    root.querySelectorAll("[data-navigation-group]").forEach((group) => {
      setModuleExpanded(group.dataset.navigationGroup, group.dataset.navigationGroup === module);
    });
    if (module === "ai-skills") {
      selectAiChild(detail ?? { section: "skills" });
    } else if (module === "deployments") {
      deploymentCatalog.section = detail?.section || null;
      deploymentCatalog.selectedId = detail?.resourceId || null;
      deploymentCatalog.selectedPlanId = detail?.planId || null;
      root.querySelectorAll("[data-deployment-section]").forEach((button) => {
        button.classList.toggle("is-active", button.dataset.deploymentSection === deploymentCatalog.section);
      });
      renderDeploymentCatalog();
    }
    onNavigate(module, root.querySelector('[data-role="module-content"]'), detail);
  };
  root.querySelectorAll(".navigation-item").forEach((item) => {
    item.addEventListener("click", () => {
      const group = root.querySelector(`[data-navigation-group="${item.dataset.module}"]`);
      if (group) {
        const opening = !group.classList.contains("is-expanded");
        setModuleExpanded(item.dataset.module, opening);
        if (opening) {
          const detail = item.dataset.module === "ai-skills"
            ? { section: "skills" }
            : item.dataset.module === "deployments" ? { section: "overview" } : null;
          navigate(item.dataset.module, detail);
        }
        return;
      }
      navigate(item.dataset.module);
    });
  });
  bindAiChildren();
  root.querySelectorAll("[data-deployment-section]").forEach((button) => {
    button.addEventListener("click", () => {
      if (button.dataset.deploymentSection === "queue" && deploymentCatalog.plans.length > 0) {
        deploymentQueueExpanded = !deploymentQueueExpanded;
        renderDeploymentCatalog();
        if (!deploymentQueueExpanded) return;
      }
      if (button.dataset.deploymentSection === "config" && deploymentCatalog.processes.length > 0) {
        deploymentConfigExpanded = !deploymentConfigExpanded;
        renderDeploymentCatalog();
        if (!deploymentConfigExpanded) return;
      }
      navigate("deployments", { section: button.dataset.deploymentSection });
    });
    button.addEventListener("contextmenu", (event) => openDeploymentContextMenu(event, button.dataset.deploymentSection));
  });
  root.querySelector('[data-action="lock"]').addEventListener("click", onLock);
  biometricButton.addEventListener("click", onBiometricToggle);

  return {
    biometricButton,
    navigate,
    setAiSkillsNavigation(libraries, skills, groups) {
      aiCatalog = { libraries, skills, groups };
      renderAiCatalog();
    },
    setDeploymentNavigation(processes, selectedId) {
      deploymentCatalog = { ...deploymentCatalog, processes, section: "config", selectedId, selectedPlanId: null };
      if (selectedId) deploymentConfigExpanded = true;
      renderDeploymentCatalog();
    },
    setDeploymentCatalog(processes) {
      deploymentCatalog.processes = processes;
      renderDeploymentCatalog();
    },
    setDeploymentQueue(plans, selectedPlanId = null) {
      deploymentCatalog.plans = plans;
      deploymentCatalog.selectedPlanId = selectedPlanId;
      if (selectedPlanId) deploymentQueueExpanded = true;
      renderDeploymentCatalog();
    },
    navigationHost(module) {
      const host = root.querySelector(`[data-role="${module}-navigation-host"]`);
      if (host) setModuleExpanded(module, true);
      return host;
    },
    showMessage(text, isError = false) {
      window.clearTimeout(messageTimer);
      message.textContent = text;
      message.classList.toggle("is-error", isError);
      message.hidden = false;
      messageTimer = window.setTimeout(() => { message.hidden = true; }, 5000);
    },
  };
}
