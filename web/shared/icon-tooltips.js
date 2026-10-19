// A shared overlay stays outside scrolling tables and survives Queue refreshes.
export function installIconTooltips() {
  const tooltip = document.createElement("div");
  tooltip.id = "gatetree-ops-icon-tooltip";
  tooltip.className = "icon-tooltip";
  tooltip.role = "tooltip";
  tooltip.hidden = true;
  document.body.append(tooltip);
  let active = null;
  let previousDescription = null;

  function hide() {
    if (active) {
      if (previousDescription) active.setAttribute("aria-describedby", previousDescription);
      else active.removeAttribute("aria-describedby");
    }
    active = null;
    tooltip.hidden = true;
  }

  function show(button) {
    if (!button?.isConnected || !button.dataset.tooltip) return hide();
    if (active !== button) {
      hide();
      active = button;
      previousDescription = button.getAttribute("aria-describedby");
      button.setAttribute("aria-describedby", [previousDescription, tooltip.id].filter(Boolean).join(" "));
    }
    tooltip.textContent = button.dataset.tooltip;
    tooltip.hidden = false;
    const rect = button.getBoundingClientRect();
    const width = tooltip.offsetWidth;
    const height = tooltip.offsetHeight;
    tooltip.style.left = `${Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8))}px`;
    tooltip.style.top = `${rect.bottom + height + 8 <= window.innerHeight ? rect.bottom + 6 : Math.max(8, rect.top - height - 6)}px`;
  }

  const buttonAt = (target) => target instanceof Element ? target.closest("button[data-tooltip]") : null;
  document.addEventListener("pointerover", (event) => show(buttonAt(event.target)), true);
  document.addEventListener("pointermove", (event) => show(buttonAt(event.target)), true);
  document.addEventListener("pointerout", (event) => {
    if (!active?.contains(event.relatedTarget)) hide();
  }, true);
  document.addEventListener("focusin", (event) => show(buttonAt(event.target)), true);
  document.addEventListener("focusout", hide, true);
  document.addEventListener("pointerdown", hide, true);
  document.addEventListener("scroll", hide, true);
  window.addEventListener("blur", hide);
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") hide(); });
}
