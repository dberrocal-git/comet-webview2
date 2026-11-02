(() => {
  'use strict';
  const catalog = window.cometI18n || {language: 'en', strings: {}};
  function t(key, parameters = {}) {
    let value = catalog.strings[key] || key;
    for (const [name, text] of Object.entries(parameters))
      value = value.split('{' + name + '}').join(String(text));
    return value;
  }
  function setHint(button, key) {
    button.title = t(key);
    button.setAttribute('aria-label', button.title);
  }
  // Keep the same DOM nodes across status updates so focus and open selectors
  // survive. Each view only supplies how to create and update its own rows.
  function profileList(container, createRow) {
    let rows = new Map();
    return (profiles, updateRow) => {
      const next = new Map();
      const nodes = profiles.map(profile => {
        const row = rows.get(profile.id) ?? createRow(profile.id);
        next.set(profile.id, row);
        updateRow(row, profile);
        return row.node;
      });
      if (nodes.length !== container.children.length ||
          nodes.some((node, i) => node !== container.children[i])) {
        container.replaceChildren(...nodes);
      }
      rows = next;
    };
  }
  function setTheme(value) {
    const theme = value === 'light' ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    for (const button of document.querySelectorAll('[data-theme-toggle]')) {
      setHint(button, theme === 'dark' ? 'theme.light' : 'theme.dark');
      button.setAttribute('aria-pressed', String(theme === 'dark'));
    }
  }
  function apply(root = document) {
    for (const node of root.querySelectorAll('[data-i18n]'))
      node.textContent = t(node.getAttribute('data-i18n'));
    for (const attribute of ['placeholder', 'title', 'aria-label']) {
      for (const node of root.querySelectorAll('[data-i18n-' + attribute + ']'))
        node.setAttribute(attribute, t(node.getAttribute('data-i18n-' + attribute)));
    }
    document.documentElement.lang = catalog.language;
    setTheme(document.documentElement.dataset.theme || window.cometSettings?.theme);
  }
  window.cometUI = Object.freeze({
    $: id => document.getElementById(id),
    ipc: message => window.ipc.postMessage(JSON.stringify(message)),
    t,
    setHint,
    setTheme,
    profileList
  });
  setTheme(window.cometSettings?.theme);
  if (document.readyState === 'loading')
    document.addEventListener('DOMContentLoaded', () => apply(), {once: true});
  else
    apply();
})();
