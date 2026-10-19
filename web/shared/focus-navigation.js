export function focusNavigationItem(root, selector) {
  // Wait until the navigation has been mounted and its expanded folders laid out.
  requestAnimationFrame(() => {
    const item = root.querySelector(selector);
    if (!item?.isConnected) return;
    item.focus({ preventScroll: true });
    item.scrollIntoView({ block: "center", inline: "nearest", behavior: "auto" });
  });
}
