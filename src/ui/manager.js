(() => {
  const {$, t, ipc, setHint, profileList} = window.cometUI;
  let current = null, version = -1, state = null, formAnimation = null;
  const renderRows = profileList($('profiles'), createProfileRow);
  const fingerprint = value => (value || '').toUpperCase().match(/.{1,2}/g)?.join(':') || '';
  function showPassword(visible) {
    $('password').type = visible ? 'text' : 'password';
    const button = $('show-password');
    button.dataset.visible = String(visible);
    setHint(button, visible ? 'password.hide' : 'password.show');
  }
  function fill(profile) {
    current = profile?.id || null;
    $('form-title').textContent = t(profile ? 'manager.connect_title' : 'manager.add');
    $('form-description').textContent =
        t(profile ? 'manager.ready_description' : 'manager.add_description');
    $('name').value = profile?.name || '';
    $('url').value = profile?.url || '';
    $('password').value = '';
    showPassword(false);
    $('remember').checked = true;
    $('password').placeholder =
        t(profile?.has_password ? 'field.saved_password_placeholder' : 'field.password_placeholder');
    $('password-hint').textContent =
        t(profile?.has_password ? 'field.saved_password_hint' : 'field.address_hint');
    $('ready').hidden = !profile?.has_password;
    $('profile-actions').hidden = !profile;
    $('profile-actions').open = false;
    $('forget').hidden = !profile?.has_password;
  }
  function createProfileRow(id) {
    const node = document.createElement('button');
    node.className = 'profile';
    const icon = document.createElement('span');
    icon.className = 'device-icon';
    icon.setAttribute('aria-hidden', 'true');
    const copy = document.createElement('span');
    copy.className = 'profile-copy';
    const name = document.createElement('b'), address = document.createElement('small');
    copy.append(name, address);
    node.append(icon, copy);
    node.onclick = () => ipc({kind: 'select_profile', id});
    return {node, name, address};
  }
  function renderProfiles(next) {
    $('profile-count').textContent = String(next.profiles.length);
    $('empty-list').hidden = next.profiles.length !== 0;
    renderRows(next.profiles, ({node, name, address}, profile) => {
      node.setAttribute('aria-selected', String(profile.id === next.selected_id));
      node.disabled = next.busy;
      name.textContent = profile.name;
      address.textContent = profile.url.replace(/^https:\/\//, '').replace(/\/$/, '');
    });
  }

  window.cometManager = Object.freeze({
    prepare() {
      document.documentElement.dataset.pagePhase = 'prepared';
    },
    enter() {
      document.documentElement.dataset.pagePhase = 'entering';
    },
    leave() {
      document.documentElement.dataset.pagePhase = 'leaving';
    },
    update(next) {
      state = next;
      renderProfiles(next);
      // Background status updates must not overwrite edits or erase a password
      // while someone is typing. Rust increments form_version to request a refill.
      if (version !== next.form_version) {
        const changed = version !== -1 && current !== (next.selected_id || null);
        version = next.form_version;
        fill(next.profiles.find(profile => profile.id === next.selected_id));
        if (changed && !$('editor').hidden &&
            !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
          formAnimation?.cancel();
          formAnimation = $('editor').animate?.(
              [{opacity: .72, transform: 'translateY(4px)'}, {opacity: 1, transform: 'translateY(0)'}],
              {duration: 160, easing: 'ease-out'});
        }
      }
      $('fields').disabled = next.busy;
      $('new').disabled = next.busy;
      $('back').hidden = !next.can_return;
      $('back').disabled = next.busy;
      $('notice').hidden = !next.notice;
      $('notice').textContent = next.notice || '';
      $('cancel').hidden = !next.busy || !!next.trust;
      $('connect').textContent = t(next.busy ? 'action.connecting' : 'action.connect');
      $('connect').dataset.loading = String(next.busy);
      $('trust').hidden = !next.trust;
      $('editor').hidden = !!next.trust;
      if (next.trust) {
        $('trust-title').textContent =
            t(next.trust.previous ? 'trust.changed_title' : 'trust.new_title');
        $('trust-origin').textContent = next.trust.url;
        $('trust-description').textContent =
            t(next.trust.previous ? 'trust.changed_description' : 'trust.new_description');
        $('trust-fingerprint').textContent = fingerprint(next.trust.fingerprint);
        $('old-certificate').hidden = !next.trust.previous;
        $('trust-previous').textContent = fingerprint(next.trust.previous);
      }
    }
  });
  function save(connect) {
    if (!$('form').reportValidity())
      return;
    ipc({
      kind: 'save_profile',
      id: current,
      name: $('name').value,
      url: $('url').value,
      password: $('password').value,
      remember: $('remember').checked,
      connect
    });
    $('password').value = '';
  }
  $('form').onsubmit = event => {
    event.preventDefault();
    save(true);
  };
  $('save').onclick = () => save(false);
  $('new').onclick = () => ipc({kind: 'new_profile'});
  $('back').onclick = () => ipc({kind: 'hide_manager'});
  $('delete').onclick = () => current && ipc({kind: 'delete_profile', id: current});
  $('forget').onclick = () => current && ipc({kind: 'forget_password', id: current});
  $('cancel').onclick = $('trust-cancel').onclick = () => ipc({kind: 'cancel_connect'});
  $('trust-accept').onclick = () => {
    if (!state?.trust) return;
    const {attempt, fingerprint} = state.trust;
    ipc({kind: 'trust_certificate', attempt, fingerprint});
  };
  $('show-password').onclick = () => showPassword($('password').type !== 'text');
  ipc({kind: 'ui_ready'});
})();
