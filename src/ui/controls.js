// Shared helpers for the bundled manager and toolbar (loaded before their scripts).
// ui::control injects cometI18n/cometSettings before either page executes. Rust is
// authoritative for persisted settings and commands; these helpers render state.
(() => {
  'use strict';
  const catalog = window.cometI18n || {language: 'en', strings: {}};
  // Literal placeholder replacement, matching ui::locale::format. Callers must
  // assign the result as text/attributes, never interpret translations as HTML.
  function t(key, parameters = {}) {
    let value = catalog.strings[key] || key;
    for (const [name, text] of Object.entries(parameters))
      value = value.split('{' + name + '}').join(String(text));
    return value;
  }
  // Icon-only controls need both a pointer tooltip and an accessible name.
  function setHint(button, key) {
    button.title = t(key);
    button.setAttribute('aria-label', button.title);
  }
  // Keep the same DOM nodes across status updates so focus and open selectors
  // survive. Native profile IDs must be unique and stable; removed IDs are evicted.
  // Each view only supplies how to create and update its own rows.
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
  // The persisted app preference controls both pages, independent of CSS media
  // queries. Also used by Rust after toggle_theme; native/WebView themes are set there.
  function setTheme(value) {
    const theme = value === 'light' ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    for (const button of document.querySelectorAll('[data-theme-toggle]')) {
      setHint(button, theme === 'dark' ? 'theme.light' : 'theme.dark');
      button.setAttribute('aria-pressed', String(theme === 'dark'));
    }
  }
  // Static labels opt in through data-i18n attributes; dynamic labels use t().
  // textContent keeps catalogs and profile-derived strings out of the HTML parser.
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
    // Command schema and per-surface permissions live in app/commands.rs.
    ipc: message => window.ipc.postMessage(JSON.stringify(message)),
    t,
    setHint,
    setTheme,
    profileList
  });
  // This runs in <head> to choose colors before the body is first painted.
  setTheme(window.cometSettings?.theme);
  if (document.readyState === 'loading')
    document.addEventListener('DOMContentLoaded', () => apply(), {once: true});
  else
    apply();
})();
