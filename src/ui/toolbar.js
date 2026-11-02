(() => {
  const {$, t, ipc, setHint, profileList} = window.cometUI;
  const button = document.querySelector('.capture'), select = $('profiles');
  let captureEnabled = window.cometSettings?.keyboard_capture !== false;
  const renderOptions = profileList(select, id => {
    const node = document.createElement('option');
    node.value = id;
    return {node};
  });
  const connect = $('connect'), pin = $('pin'), fullscreen = document.querySelector('.fullscreen');
  let viewerId = null, online = false, busy = false, managerVisible = true;
  let autoHide = false, collapsed = false, hideTimer;

  function renderConnection() {
    const connected = !!viewerId && select.value === viewerId && online;
    connect.textContent = t(connected ? 'toolbar.connected' : 'action.connect');
    connect.dataset.connected = String(connected);
    connect.disabled = busy || connected || !select.value;
  }
  function reveal() {
    clearTimeout(hideTimer);
    if (autoHide && collapsed)
      ipc({kind: 'reveal_toolbar', visible: true});
  }
  function hideAfterLeave() {
    clearTimeout(hideTimer);
    if (!autoHide || collapsed)
      return;
    hideTimer = setTimeout(() => {
      if (document.documentElement.matches(':hover'))
        return;
      if (window.CSS?.supports('selector(select:open)') && select.matches(':open')) {
        hideAfterLeave();
        return;
      }
      ipc({kind: 'reveal_toolbar', visible: false});
    }, 400);
  }
  function setCaptured(captured, basic = false) {
    let label = captureEnabled ? 'capture.ready' : 'capture.off';
    if (captured) label = 'capture.active';
    else if (basic) label = 'capture.basic';
    $('capture-label').textContent = t(label);
    button.dataset.active = String(captured);
    button.setAttribute('aria-pressed', String(captureEnabled));
    setHint(button, captureEnabled ? 'capture.enabled_hint' : 'capture.disabled_hint');
  }
  window.cometToolbar = Object.freeze({
    layout(state) {
      autoHide = state.autoHide;
      collapsed = state.collapsed;
      document.documentElement.dataset.collapsed = String(collapsed);
      setHint(pin, state.pinned ? 'toolbar.unpin' : 'toolbar.pin');
      pin.setAttribute('aria-pressed', String(state.pinned));
      setHint(fullscreen, state.fullscreen ? 'toolbar.exit_fullscreen' : 'toolbar.fullscreen');
      if (!autoHide || collapsed)
        clearTimeout(hideTimer);
      else if (!document.documentElement.matches(':hover'))
        hideAfterLeave();
    },
    update(state) {
      online = !!state.online;
      renderConnection();
      captureEnabled = state.capture_enabled !== false;
      setCaptured(state.captured, state.basic);
      button.disabled = !state.width;
    },
    profiles(state) {
      const previous = select.value;
      const enteringSession = managerVisible && !state.manager_visible;
      const viewerChanged = viewerId !== (state.viewer_id || null);
      viewerId = state.viewer_id || null;
      managerVisible = state.manager_visible;
      busy = state.busy;
      renderOptions(state.profiles, ({node}, profile) => {
        if (node.textContent !== profile.name) node.textContent = profile.name;
      });
      const hasProfile = id => !!id && state.profiles.some(profile => profile.id === id);
      const keepChoice = !enteringSession && !viewerChanged && hasProfile(previous);
      select.value = keepChoice ? previous :
          [viewerId, state.selected_id, state.profiles[0]?.id].find(hasProfile) || '';
      select.disabled = busy || !state.profiles.length;
      pin.hidden = managerVisible;
      renderConnection();
      $('connection-group').hidden = state.manager_visible;
      $('remote-controls').hidden = state.manager_visible;
      $('section').hidden = !state.manager_visible;
    }
  });
  button.onclick = () => ipc({kind: 'capture', enabled: !captureEnabled});
  fullscreen.onclick = () => ipc({kind: 'fullscreen'});
  pin.onclick = () => ipc({kind: 'toggle_toolbar_pin'});
  $('reveal').onclick = reveal;
  document.documentElement.onpointerenter = reveal;
  document.documentElement.onpointerleave = hideAfterLeave;
  $('manage').onclick = () => ipc({kind: 'show_manager'});
  select.onchange = renderConnection;
  connect.onclick = () => {
    if (!connect.disabled)
      ipc({kind: 'connect_profile', id: select.value});
  };
  $('theme-toggle').onclick = () => ipc({kind: 'toggle_theme'});
  document.addEventListener('pointerdown', event => {
    if (select.contains(event.target))
      ipc({kind: 'suspend_capture'});
  }, true);
  setCaptured(false);
  ipc({kind: 'ui_ready'});
})();
