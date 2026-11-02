// Document-start adapter between native capture and the device's existing UI.
// ui::remote injects the configured origin before this script. Keep this early:
// socket observation and DOM interception must precede firmware initialization.
//
// Host API: __cometNative.setCaptured(enabled, useDom), batch([[code, down], ...]),
// key(code, down), release(). Rust owns permission to capture; console clicks only
// request it through IPC. Firmware chooses the session's HID transport.
(() => {
  'use strict';
  // Navigation and IPC are checked again in Rust. This guard also prevents
  // bridge installation in subframes and pages at a different port/origin.
  if (window.top !== window || location.origin !== window.__cometConnectionOrigin)
    return;
  const ipc = message => window.ipc.postMessage(JSON.stringify(message));
  // Only keys actually sent remotely enter this ledger. Release is idempotent,
  // including when a queued physical key-up arrives after native capture ended.
  const held = new Set();
  // Observe the device's control connection, which stays open even without HDMI.
  const openControlSockets = new Set();
  window.WebSocket = new Proxy(window.WebSocket, {
    construct(target, args, newTarget) {
      const socket = Reflect.construct(target, args, newTarget);
      const address = new URL(socket.url);
      if (address.protocol === 'wss:' && address.host === location.host &&
          address.pathname === '/api/ws') {
        socket.addEventListener('open', () => openControlSockets.add(socket), {once: true});
        socket.addEventListener('close', () => openControlSockets.delete(socket), {once: true});
      }
      return socket;
    },
  });
  // captured is host permission, basic selects trusted DOM typing when exclusive
  // acquisition failed, and consoleSelected suppresses firmware DOM handling even
  // when the user has disabled capture. Pointer movement changes none of them.
  let captured = false, basic = false, consoleSelected = false;
  let hidPackets = 0, browserLockRequests = 0, domKeyEvents = 0;
  let lastFrames = 0, lastTime = performance.now();
  // Native capture owns system shortcuts. Chromium Keyboard Lock installs a
  // competing OS hook when the device frontend enters fullscreen.
  if (typeof navigator !== 'undefined' && navigator.keyboard) {
    try {
      navigator.keyboard.unlock?.();
      Object.defineProperty(navigator.keyboard, 'lock', {
        value: () => {
          browserLockRequests++;
          return Promise.resolve();
        }
      });
    } catch { /* Some WebView2 versions expose a read-only Keyboard Lock API. */ }
  }
  // These counters are diagnostic heuristics over outgoing binary packets. They
  // do not acknowledge remote key delivery or choose a transport. Preserve return
  // values/receivers when wrapping send so firmware networking behaves normally.
  const countHid = data => {
    if (ArrayBuffer.isView(data) && data.byteLength > 2 &&
        new Uint8Array(data.buffer, data.byteOffset, 1)[0] === 1)
      hidPackets++;
  };
  const wsSend = WebSocket.prototype.send;
  WebSocket.prototype.send = function(data) {
    countHid(data);
    return wsSend.call(this, data);
  };
  if (typeof RTCDataChannel !== 'undefined') {
    const dcSend = RTCDataChannel.prototype.send;
    RTCDataChannel.prototype.send = function(data) {
      countHid(data);
      return dcSend.call(this, data);
    };
  }
  function send(code, down) {
    // The firmware's desktop bridge chooses the HID transport for its session.
    // Keep physical key codes intact so the remote Windows layout interprets them.
    window.postMessage(
        JSON.stringify({action: 'remote_keyboard_event', payload: {key: code, state: down}}),
        location.origin);
  }
  // Send breaks for every outstanding make before forgetting our remote state.
  function release() {
    for (const code of held)
      send(code, false);
    held.clear();
  }
  // A new session is reset explicitly by the host with false before true. Calling
  // true alone keeps held keys, so callers must not use it as a reset operation.
  function setCaptured(value, useDom = false) {
    captured = value;
    basic = value && useDom;
    if (!value)
      release();
  }
  // Firmware DOM coupling: containers cover overlays and the no-HDMI console;
  // editors/dialogs inside them must keep ordinary local typing. Recheck these
  // selectors when supporting firmware with a different console structure.
  function isVideo(target) {
    if (target?.closest?.('input,textarea,select,[contenteditable="true"],[role="dialog"]'))
      return false;
    return !!target &&
        (target.tagName === 'VIDEO' || !!target.closest?.('#video-wrapper, #stream-box'));
  }
  // Deduplicate make/break pairs from either native batches or basic DOM input.
  // Ignore new downs after capture ends, while still allowing known held ups.
  function key(code, down) {
    if (down && !captured)
      return;
    if (down) {
      if (held.has(code))
        return;
      held.add(code);
    } else if (!held.delete(code))
      return;
    send(code, down);
  }
  // Fixed host/bridge contract, not an isolation boundary from same-origin device
  // scripts. Preserve batch order so modifier chords retain their physical order.
  Object.defineProperty(window, '__cometNative', {
    value: Object.freeze({
      key,
      release,
      setCaptured,
      batch(events) {
        if (!captured) return;
        for (const [code, down] of events) key(code, down);
      },
    })
  });
  // The native host owns capture. Disable the firmware's duplicate DOM path
  // over the console, including when the manual capture switch is off.
  for (const type of ['keydown', 'keyup'])
    window.addEventListener(type, event => {
      if (captured || consoleSelected || isVideo(event.target)) {
        // Only the fallback uses DOM keys. Exclusive input owns its entire stream.
        if (basic && event.isTrusted && event.code) {
          domKeyEvents++;
          key(event.code, type === 'keydown');
        }
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    }, true);
  document.addEventListener('pointerdown', event => {
    consoleSelected = isVideo(event.target);
    ipc({kind: 'capture', enabled: consoleSelected});
  }, true);
  // The host releases on top-level focus loss. Child WebViews can blur during a click.
  window.addEventListener('pagehide', () => {
    setCaptured(false);
    ipc({kind: 'capture', enabled: false});
  });
  // Connectivity is deliberately independent of video dimensions: an open device
  // control socket permits keyboard capture even when no HDMI source is present.
  // Rust uses these periodic stats to end capture when the session disconnects.
  function mount() {
    ipc({kind: 'ready'});
    setInterval(() => {
      const video = document.querySelector('video'), now = performance.now();
      const frames = video?.getVideoPlaybackQuality?.().totalVideoFrames || 0;
      const fps =
          video && frames >= lastFrames ? (frames - lastFrames) * 1000 / (now - lastTime) : 0;
      lastFrames = frames;
      lastTime = now;
      const ready = video && video.readyState >= 2;
      const width = ready ? video.videoWidth : 0, height = ready ? video.videoHeight : 0;
      ipc({
        kind: 'stats',
        width,
        height,
        fps,
        connected: openControlSockets.size > 0 || fps > 0,
        hid_packets: hidPackets,
        browser_lock_requests: browserLockRequests,
        dom_key_events: domKeyEvents
      });
    }, 1000);
  }
  if (document.readyState === 'loading')
    document.addEventListener('DOMContentLoaded', mount, {once: true});
  else
    mount();
})();
