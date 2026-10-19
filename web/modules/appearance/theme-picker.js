const THEME_KEY = "gatetree-ops.appearance.theme";

const themes = [
  ["ocean", "Ocean", "#58d0d5"],
  ["graphite", "Graphite", "#a8b3c2"],
  ["pearl", "Pearl", "#16818b"],
  ["linen", "Linen", "#a5672c"],
];

export function initializeTheme() {
  const saved = localStorage.getItem(THEME_KEY);
  const selected = themes.some(([id]) => id === saved) ? saved : "ocean";
  document.documentElement.dataset.theme = selected;
}

export function themePickerMarkup() {
  return `
    <div class="theme-control">
      <button class="icon-button theme-button" data-action="toggle-theme-picker" aria-label="Choose appearance" title="Choose appearance">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a9 9 0 1 0 0 18h1.2a1.8 1.8 0 0 0 0-3.6h-.7a1.7 1.7 0 0 1 0-3.4H15a6 6 0 0 0 0-12h-3Z"/><circle cx="7.5" cy="10" r=".7"/><circle cx="9.5" cy="6.8" r=".7"/><circle cx="14" cy="6.5" r=".7"/></svg>
      </button>
      <div class="theme-picker" data-role="theme-picker" hidden>
        <strong>Appearance</strong>
        ${themes.map(([id, label, color]) => `
          <button data-theme-option="${id}">
            <i style="--swatch:${color}"></i>
            <span>${label}</span>
            <b aria-hidden="true">✓</b>
          </button>`).join("")}
      </div>
    </div>`;
}

export function bindThemePicker(root) {
  const toggle = root.querySelector('[data-action="toggle-theme-picker"]');
  const picker = root.querySelector('[data-role="theme-picker"]');

  const updateSelection = () => {
    picker.querySelectorAll("[data-theme-option]").forEach((button) => {
      button.classList.toggle("is-active", button.dataset.themeOption === document.documentElement.dataset.theme);
    });
  };
  updateSelection();

  toggle.addEventListener("click", () => {
    picker.hidden = !picker.hidden;
    if (!picker.hidden) updateSelection();
  });
  picker.querySelectorAll("[data-theme-option]").forEach((button) => {
    button.addEventListener("click", () => {
      const theme = button.dataset.themeOption;
      document.documentElement.dataset.theme = theme;
      localStorage.setItem(THEME_KEY, theme);
      updateSelection();
      picker.hidden = true;
    });
  });
}
