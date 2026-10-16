export function createModuleRoot(contentRoot, navigationRoot = null) {
  if (!navigationRoot) return contentRoot;
  return {
    contentRoot,
    navigationRoot,
    get dataset() { return contentRoot.dataset; },
    get innerHTML() { return contentRoot.innerHTML; },
    set innerHTML(value) {
      navigationRoot.replaceChildren();
      contentRoot.innerHTML = value;
    },
    querySelector(selector) {
      return contentRoot.querySelector(selector) ?? navigationRoot.querySelector(selector);
    },
    querySelectorAll(selector) {
      return [...contentRoot.querySelectorAll(selector), ...navigationRoot.querySelectorAll(selector)];
    },
    mountNavigation(selector) {
      const element = contentRoot.querySelector(selector);
      if (element) navigationRoot.append(element);
    },
  };
}
