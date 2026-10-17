/**
 * Arduracer PSX Web Arcade Companion Controller & Emulator Bridge
 */

document.addEventListener('DOMContentLoaded', () => {
  const iframe = document.getElementById('emulator-frame');
  const crtOverlay = document.getElementById('crt-overlay');
  const crtSelect = document.getElementById('crt-select');
  const discSelect = document.getElementById('disc-select');
  const btnReset = document.getElementById('btn-reset');
  const btnMute = document.getElementById('btn-mute');
  const btnFullscreen = document.getElementById('btn-fullscreen');
  const statusLed = document.getElementById('status-led');
  const statusText = document.getElementById('status-text');
  const gamepadStatus = document.getElementById('gamepad-status');
  const gamepadName = document.getElementById('gamepad-name');

  // Gamepad HUD SVG elements
  const svgElements = {
    cross: document.getElementById('svg-cross'),
    circle: document.getElementById('svg-circle'),
    square: document.getElementById('svg-square'),
    triangle: document.getElementById('svg-triangle'),
    up: document.getElementById('svg-dpad-up'),
    down: document.getElementById('svg-dpad-down'),
    left: document.getElementById('svg-dpad-left'),
    right: document.getElementById('svg-dpad-right'),
    l1: document.getElementById('svg-l1'),
    r1: document.getElementById('svg-r1'),
    start: document.getElementById('svg-start'),
    select: document.getElementById('svg-select'),
    stickL: document.getElementById('svg-stick-l'),
    stickR: document.getElementById('svg-stick-r')
  };

  function setButtonState(name, active) {
    const el = svgElements[name];
    if (el) {
      if (active) {
        el.setAttribute('fill', '#00d2ff');
        el.setAttribute('filter', 'drop-shadow(0 0 6px #00d2ff)');
      } else {
        el.removeAttribute('fill');
        el.removeAttribute('filter');
      }
    }
  }

  let isMuted = false;
  let isRunning = false;

  // 1. PostMessage Bridge to PSoXide Player
  function sendEmulatorCommand(command) {
    if (iframe && iframe.contentWindow) {
      iframe.contentWindow.postMessage(
        { type: 'psoxide-command', command: command },
        window.location.origin
      );
    }
  }

  window.addEventListener('message', (e) => {
    if (e.origin !== window.location.origin) return;
    const data = e.data;
    if (!data || data.type !== 'psoxide-event') return;

    console.log('[PSoXide Event]', data.event, data.message || '');
    switch (data.event) {
      case 'ready':
        statusLed.style.background = '#f59e0b';
        statusLed.style.boxShadow = '0 0 8px #f59e0b';
        statusText.textContent = 'READY (CLICK SCREEN TO START)';
        break;
      case 'starting':
        statusLed.style.background = '#3b82f6';
        statusLed.style.boxShadow = '0 0 8px #3b82f6';
        statusText.textContent = 'BOOTING MIPS R3000...';
        break;
      case 'running':
        isRunning = true;
        statusLed.style.background = '#10b981';
        statusLed.style.boxShadow = '0 0 8px #10b981';
        statusText.textContent = 'RUNNING (60 FPS)';
        break;
      case 'paused':
        isRunning = false;
        statusLed.style.background = '#f59e0b';
        statusLed.style.boxShadow = '0 0 8px #f59e0b';
        statusText.textContent = 'PAUSED';
        break;
      case 'error':
        statusLed.style.background = '#ef4444';
        statusLed.style.boxShadow = '0 0 8px #ef4444';
        statusText.textContent = `ERROR: ${data.message || 'EMULATION HALTED'}`;
        break;
    }
  });

  // 2. Toolbar Actions
  if (btnReset) {
    btnReset.addEventListener('click', () => {
      sendEmulatorCommand('reset');
      statusText.textContent = 'RESETTING...';
    });
  }

  if (btnMute) {
    btnMute.addEventListener('click', () => {
      isMuted = !isMuted;
      sendEmulatorCommand(isMuted ? 'mute' : 'unmute');
      btnMute.textContent = isMuted ? '🔇 Unmute' : '🔊 Mute';
      btnMute.classList.toggle('active', isMuted);
    });
  }

  if (btnFullscreen) {
    btnFullscreen.addEventListener('click', () => {
      const target = (touchEnabled && isMobile())
        ? (document.querySelector('.arcade-bezel') || document.querySelector('.screen-container'))
        : document.querySelector('.screen-container');
      if (!document.fullscreenElement) {
        if (target.requestFullscreen) target.requestFullscreen();
        else if (target.webkitRequestFullscreen) target.webkitRequestFullscreen();
      } else {
        if (document.exitFullscreen) document.exitFullscreen();
      }
    });
  }

  // --- PS1 Mobile On-Screen Touch Controller ---
  const btnTouch = document.getElementById('btn-touch');
  const touchControls = document.getElementById('touch-controls');
  const touchDpad = document.getElementById('touch-dpad');

  const PS_PAD = {
    SELECT:   1 << 0,  // 0x0001
    L3:       1 << 1,  // 0x0002
    R3:       1 << 2,  // 0x0004
    START:    1 << 3,  // 0x0008
    UP:       1 << 4,  // 0x0010
    RIGHT:    1 << 5,  // 0x0020
    DOWN:     1 << 6,  // 0x0040
    LEFT:     1 << 7,  // 0x0080
    L2:       1 << 8,  // 0x0100
    R2:       1 << 9,  // 0x0200
    L1:       1 << 10, // 0x0400
    R1:       1 << 11, // 0x0800
    TRIANGLE: 1 << 12, // 0x1000
    CIRCLE:   1 << 13, // 0x2000
    CROSS:    1 << 14, // 0x4000
    SQUARE:   1 << 15  // 0x8000
  };

  const BUTTON_CONFIG = {
    up:       { mask: PS_PAD.UP,       key: 'ArrowUp',    code: 'ArrowUp' },
    down:     { mask: PS_PAD.DOWN,     key: 'ArrowDown',  code: 'ArrowDown' },
    left:     { mask: PS_PAD.LEFT,     key: 'ArrowLeft',  code: 'ArrowLeft' },
    right:    { mask: PS_PAD.RIGHT,    key: 'ArrowRight', code: 'ArrowRight' },
    cross:    { mask: PS_PAD.CROSS,    key: 'x',          code: 'KeyX' },
    circle:   { mask: PS_PAD.CIRCLE,   key: 'c',          code: 'KeyC' },
    square:   { mask: PS_PAD.SQUARE,   key: 'z',          code: 'KeyZ' },
    triangle: { mask: PS_PAD.TRIANGLE, key: 'v',          code: 'KeyV' },
    l1:       { mask: PS_PAD.L1,       key: 'q',          code: 'KeyQ' },
    r1:       { mask: PS_PAD.R1,       key: 'e',          code: 'KeyE' },
    select:   { mask: PS_PAD.SELECT,   key: 'Tab',        code: 'Tab' },
    start:    { mask: PS_PAD.START,    key: 'Enter',      code: 'Enter' }
  };

  const isMobile = () => {
    return (
      'ontouchstart' in window ||
      navigator.maxTouchPoints > 0 ||
      window.matchMedia('(pointer: coarse)').matches ||
      window.innerWidth <= 900
    );
  };

  let touchEnabled = false;
  const savedTouchPref = localStorage.getItem('arduracer_touch_controls');
  if (savedTouchPref === 'enabled') {
    touchEnabled = true;
  } else if (savedTouchPref === 'disabled') {
    touchEnabled = false;
  } else {
    touchEnabled = isMobile();
  }

  function updateTouchVisibility() {
    if (touchControls) {
      touchControls.classList.toggle('touch-hidden', !touchEnabled);
    }
    if (btnTouch) {
      btnTouch.classList.toggle('active', touchEnabled);
      btnTouch.textContent = touchEnabled ? '📱 Touch (On)' : '📱 Touch Controls';
    }
  }
  updateTouchVisibility();

  if (btnTouch) {
    btnTouch.addEventListener('click', () => {
      touchEnabled = !touchEnabled;
      localStorage.setItem('arduracer_touch_controls', touchEnabled ? 'enabled' : 'disabled');
      updateTouchVisibility();
    });
  }

  let activeTouchButtons = new Set();
  let currentPadMask = 0;

  function dispatchKeyEvent(code, key, type) {
    if (!iframe || !iframe.contentWindow) return;
    try {
      const doc = iframe.contentDocument || iframe.contentWindow.document;
      const ev = new KeyboardEvent(type, {
        code: code,
        key: key,
        bubbles: true,
        cancelable: true
      });
      if (doc) doc.dispatchEvent(ev);
      iframe.contentWindow.dispatchEvent(ev);
    } catch (_) {}
  }

  function applyActiveButtons(newActiveButtons) {
    let newMask = 0;
    const newlyPressed = [];

    for (const [btnName, cfg] of Object.entries(BUTTON_CONFIG)) {
      const wasActive = activeTouchButtons.has(btnName);
      const isActive = newActiveButtons.has(btnName);

      if (isActive) {
        newMask |= cfg.mask;
      }

      if (isActive && !wasActive) {
        newlyPressed.push(btnName);
        dispatchKeyEvent(cfg.code, cfg.key, 'keydown');
        if (typeof setButtonState === 'function') setButtonState(btnName, true);
      } else if (!isActive && wasActive) {
        dispatchKeyEvent(cfg.code, cfg.key, 'keyup');
        if (typeof setButtonState === 'function') setButtonState(btnName, false);
      }
    }

    // Gentle tactile haptic feedback
    if (newlyPressed.length > 0 && navigator.vibrate) {
      try { navigator.vibrate(12); } catch (_) {}
    }

    // Update DOM visual active states
    if (touchControls) {
      const buttons = touchControls.querySelectorAll('[data-btn]');
      buttons.forEach((b) => {
        const name = b.dataset.btn;
        b.classList.toggle('active', newActiveButtons.has(name));
      });
    }

    // Bridge mask to emulator iframe
    currentPadMask = newMask;
    if (iframe && iframe.contentWindow) {
      try {
        iframe.contentWindow._psoxideVirtualPadMask = newMask;
      } catch (_) {}
      try {
        iframe.contentWindow.postMessage({ type: 'psoxide-pad', mask: newMask }, window.location.origin);
      } catch (_) {}
    }

    activeTouchButtons = newActiveButtons;
  }

  if (touchControls) {
    function processTouchInput(e) {
      if (!touchEnabled) return;
      if (e.cancelable) e.preventDefault();

      const newButtons = new Set();
      const touches = e.touches;

      for (let i = 0; i < touches.length; i++) {
        const touch = touches[i];

        // Check D-pad analog tracking
        if (touchDpad) {
          const rect = touchDpad.getBoundingClientRect();
          const margin = 14;
          if (
            touch.clientX >= rect.left - margin &&
            touch.clientX <= rect.right + margin &&
            touch.clientY >= rect.top - margin &&
            touch.clientY <= rect.bottom + margin
          ) {
            const cx = rect.left + rect.width / 2;
            const cy = rect.top + rect.height / 2;
            const dx = touch.clientX - cx;
            const dy = touch.clientY - cy;
            const dist = Math.hypot(dx, dy);

            if (dist >= 12) {
              if (dy < -14) newButtons.add('up');
              if (dy > 14) newButtons.add('down');
              if (dx < -14) newButtons.add('left');
              if (dx > 14) newButtons.add('right');
            }
            continue;
          }
        }

        // Check discrete button hit
        const el = document.elementFromPoint(touch.clientX, touch.clientY);
        const btn = el ? el.closest('[data-btn]') : null;
        if (btn && btn.dataset.btn) {
          newButtons.add(btn.dataset.btn);
        }
      }

      applyActiveButtons(newButtons);
    }

    touchControls.addEventListener('touchstart', processTouchInput, { passive: false });
    touchControls.addEventListener('touchmove', processTouchInput, { passive: false });
    touchControls.addEventListener('touchend', processTouchInput, { passive: false });
    touchControls.addEventListener('touchcancel', processTouchInput, { passive: false });

    // Desktop mouse fallback for testing
    let isMouseDown = false;

    touchControls.addEventListener('mousedown', (e) => {
      const btn = e.target.closest('[data-btn]');
      if (btn && btn.dataset.btn) {
        isMouseDown = true;
        const set = new Set(activeTouchButtons);
        set.add(btn.dataset.btn);
        applyActiveButtons(set);
      }
    });

    window.addEventListener('mouseup', () => {
      if (isMouseDown) {
        isMouseDown = false;
        applyActiveButtons(new Set());
      }
    });
  }

  // 3. CRT Shader Selection
  if (crtSelect) {
    crtSelect.addEventListener('change', (e) => {
      const mode = e.target.value;
      crtOverlay.className = 'crt-overlay';
      if (mode === 'scanlines') {
        crtOverlay.classList.add('crt-scanlines');
      } else if (mode === 'trinitron') {
        crtOverlay.classList.add('crt-trinitron');
      }
    });
  }

  // 4. Disc Image Switcher
  if (discSelect) {
    discSelect.addEventListener('change', (e) => {
      const discPath = e.target.value;
      iframe.src = `player/?embed=1&disc=${discPath}`;
      statusLed.style.background = '#f59e0b';
      statusText.textContent = 'RELOADING DISC...';
      if (typeof refreshCacheUI === 'function') refreshCacheUI();
    });
  }

  // 5. Interactive Tab Navigation
  const tabBtns = document.querySelectorAll('.tab-btn');
  const tabContents = document.querySelectorAll('.tab-content');

  tabBtns.forEach((btn) => {
    btn.addEventListener('click', () => {
      const targetId = btn.getAttribute('data-tab');

      tabBtns.forEach((b) => b.classList.remove('active'));
      tabContents.forEach((c) => c.classList.remove('active'));

      btn.classList.add('active');
      const targetEl = document.getElementById(targetId);
      if (targetEl) targetEl.classList.add('active');
    });
  });

  // 6. Keyboard mapping mirror for visualization
  const keyMap = {
    'KeyW': 'up',
    'ArrowUp': 'up',
    'KeyS': 'down',
    'ArrowDown': 'down',
    'KeyA': 'left',
    'ArrowLeft': 'left',
    'KeyD': 'right',
    'ArrowRight': 'right',
    'KeyK': 'cross',
    'KeyX': 'cross',
    'KeyL': 'circle',
    'KeyC': 'circle',
    'KeyJ': 'square',
    'KeyZ': 'square',
    'KeyI': 'triangle',
    'KeyV': 'triangle',
    'KeyQ': 'l1',
    'KeyE': 'r1',
    'Enter': 'start',
    'ShiftRight': 'select',
    'ShiftLeft': 'select',
    'Tab': 'select'
  };

  window.addEventListener('keydown', (e) => {
    const btn = keyMap[e.code];
    if (btn) setButtonState(btn, true);
  });

  window.addEventListener('keyup', (e) => {
    const btn = keyMap[e.code];
    if (btn) setButtonState(btn, false);
  });

  // Gamepad polling loop
  window.addEventListener('gamepadconnected', (e) => {
    if (gamepadStatus) gamepadStatus.classList.add('connected');
    if (gamepadName) gamepadName.textContent = e.gamepad.id.slice(0, 32);
  });

  window.addEventListener('gamepaddisconnected', () => {
    if (gamepadStatus) gamepadStatus.classList.remove('connected');
    if (gamepadName) gamepadName.textContent = 'No Gamepad Detected (Keyboard Ready)';
  });

  function pollGamepads() {
    const gamepads = navigator.getGamepads ? navigator.getGamepads() : [];
    if (gamepads && gamepads[0]) {
      const gp = gamepads[0];
      if (gamepadStatus && !gamepadStatus.classList.contains('connected')) {
        gamepadStatus.classList.add('connected');
        gamepadName.textContent = gp.id.slice(0, 32);
      }

      // Standard PS/Xbox button mapping
      setButtonState('cross', gp.buttons[0] && gp.buttons[0].pressed);
      setButtonState('circle', gp.buttons[1] && gp.buttons[1].pressed);
      setButtonState('square', gp.buttons[2] && gp.buttons[2].pressed);
      setButtonState('triangle', gp.buttons[3] && gp.buttons[3].pressed);
      setButtonState('l1', gp.buttons[4] && gp.buttons[4].pressed);
      setButtonState('r1', gp.buttons[5] && gp.buttons[5].pressed);
      setButtonState('select', gp.buttons[8] && gp.buttons[8].pressed);
      setButtonState('start', gp.buttons[9] && gp.buttons[9].pressed);
      setButtonState('up', gp.buttons[12] && gp.buttons[12].pressed || gp.axes[1] < -0.4);
      setButtonState('down', gp.buttons[13] && gp.buttons[13].pressed || gp.axes[1] > 0.4);
      setButtonState('left', gp.buttons[14] && gp.buttons[14].pressed || gp.axes[0] < -0.4);
      setButtonState('right', gp.buttons[15] && gp.buttons[15].pressed || gp.axes[0] > 0.4);

      if (svgElements.stickL && gp.axes.length >= 2) {
        const dx = Math.round(gp.axes[0] * 8);
        const dy = Math.round(gp.axes[1] * 8);
        svgElements.stickL.setAttribute('transform', `translate(${dx}, ${dy})`);
      }
    }
    requestAnimationFrame(pollGamepads);
  }
  // 7. ROM Cache Manager UI & Event Hooks
  const btnCache = document.getElementById('btn-cache');
  const cacheBadge = document.getElementById('cache-badge');
  const cacheModal = document.getElementById('cache-modal');
  const btnCloseCache = document.getElementById('btn-close-cache');
  const btnPrecache = document.getElementById('btn-precache');
  const btnCheckUpdates = document.getElementById('btn-check-updates');
  const btnFlushCache = document.getElementById('btn-flush-cache');
  const cacheFileList = document.getElementById('cache-file-list');
  const cacheBuildInfo = document.getElementById('cache-build-info');
  const cacheProgressContainer = document.getElementById('cache-progress-container');
  const cacheProgressFilename = document.getElementById('cache-progress-filename');
  const cacheProgressPercent = document.getElementById('cache-progress-percent');
  const cacheProgressBarFill = document.getElementById('cache-progress-bar-fill');

  const ROM_FILES = [
    { name: 'arduracer.cue', path: 'roms/arduracer.cue', desc: 'Cue Sheet' },
    { name: 'arduracer.bin', path: 'roms/arduracer.bin', desc: 'Track 1 Data + CDDA Audio' },
    { name: 'arduracer.iso', path: 'roms/arduracer.iso', desc: 'Compact Disc ISO' },
    { name: 'arduracer.exe', path: 'roms/arduracer.exe', desc: 'Standalone Executable' }
  ];

  async function refreshCacheUI() {
    if (!window.romCache) return;
    const status = await window.romCache.getStatus();

    if (cacheBuildInfo) {
      cacheBuildInfo.textContent = `BUILD ID: ${status.buildId || 'local'} • TOTAL CACHED: ${RomCacheManager.formatBytes(status.totalBytes)}`;
    }

    // Determine current disc status
    const currentDiscVal = discSelect ? discSelect.value : '../roms/arduracer.cue';
    const currentFilename = currentDiscVal.split('/').pop();
    const isCurrentCached = status.files.some((f) => f.filename === currentFilename);

    if (cacheBadge) {
      if (isCurrentCached) {
        cacheBadge.className = 'cache-badge cached';
        cacheBadge.textContent = 'Cached';
      } else {
        cacheBadge.className = 'cache-badge streaming';
        cacheBadge.textContent = 'Stream';
      }
    }

    if (cacheFileList) {
      cacheFileList.innerHTML = '';
      ROM_FILES.forEach((item) => {
        const cachedItem = status.files.find((f) => f.filename === item.name);
        const el = document.createElement('div');
        el.className = 'cache-file-item';
        const isCached = !!cachedItem;
        const sizeStr = isCached ? RomCacheManager.formatBytes(cachedItem.size) : '';

        el.innerHTML = `
          <span><strong>${item.name}</strong> <span style="color:#64748b;">(${item.desc})</span></span>
          <span class="cache-file-status" style="color: ${isCached ? '#34d399' : '#64748b'};">
            ${isCached ? `✓ Cached (${sizeStr})` : '○ On-Demand'}
          </span>
        `;
        cacheFileList.appendChild(el);
      });
    }
  }

  if (window.romCache) {
    window.romCache.init().then(() => {
      refreshCacheUI();
    });

    window.romCache.subscribe((event, data) => {
      if (event === 'new-image') {
        statusLed.style.background = '#f59e0b';
        statusText.textContent = 'NEW DISC IMAGE DETECTED • CACHE FLUSHED';
        refreshCacheUI();
      } else if (event === 'cache-flushed') {
        statusLed.style.background = '#f59e0b';
        statusText.textContent = 'DISC CACHE FLUSHED';
        refreshCacheUI();
      } else if (event === 'status-change') {
        refreshCacheUI();
      } else if (event === 'progress') {
        if (cacheProgressContainer) {
          cacheProgressContainer.hidden = false;
          if (cacheProgressFilename) {
            const fname = data.url.split('/').pop();
            cacheProgressFilename.textContent = `Downloading ${fname}: ${RomCacheManager.formatBytes(data.loaded)} / ${RomCacheManager.formatBytes(data.total)}`;
          }
          if (cacheProgressPercent) {
            cacheProgressPercent.textContent = `${data.percent}%`;
          }
          if (cacheProgressBarFill) {
            cacheProgressBarFill.style.width = `${data.percent}%`;
          }
        }
        if (cacheBadge) {
          cacheBadge.className = 'cache-badge downloading';
          cacheBadge.textContent = `${data.percent}%`;
        }
      }
    });
  }

  if (btnCache) {
    btnCache.addEventListener('click', () => {
      if (cacheModal) {
        cacheModal.hidden = !cacheModal.hidden;
        if (!cacheModal.hidden) refreshCacheUI();
      }
    });
  }

  if (btnCloseCache) {
    btnCloseCache.addEventListener('click', () => {
      if (cacheModal) cacheModal.hidden = true;
    });
  }

  if (btnCheckUpdates) {
    btnCheckUpdates.addEventListener('click', async () => {
      btnCheckUpdates.disabled = true;
      btnCheckUpdates.textContent = 'Checking...';
      const updated = await window.romCache.checkForUpdates();
      btnCheckUpdates.disabled = false;
      btnCheckUpdates.textContent = '🔄 Check Updates';
      if (updated) {
        statusText.textContent = 'NEW IMAGES FOUND • CACHE UPDATED';
      } else {
        statusText.textContent = 'DISC IMAGES UP TO DATE';
      }
      refreshCacheUI();
    });
  }

  if (btnFlushCache) {
    btnFlushCache.addEventListener('click', async () => {
      if (confirm('Flush all cached ROM disc images? Future runs will stream or re-download.')) {
        await window.romCache.flushCache();
        refreshCacheUI();
      }
    });
  }

  if (btnPrecache) {
    btnPrecache.addEventListener('click', async () => {
      const currentDiscVal = discSelect ? discSelect.value : '../roms/arduracer.cue';
      const cleanPath = currentDiscVal.replace(/^\.\.\//, '');
      btnPrecache.disabled = true;
      btnPrecache.textContent = 'Caching...';
      statusText.textContent = `PRE-CACHING ${cleanPath.toUpperCase()}...`;

      try {
        if (cleanPath.endsWith('.cue')) {
          await window.romCache.cacheRom('roms/arduracer.cue');
          await window.romCache.cacheRom('roms/arduracer.bin');
        } else {
          await window.romCache.cacheRom(cleanPath);
        }
        statusText.textContent = 'PRE-CACHE COMPLETE • READY TO RUN';
      } catch (err) {
        console.error('Precache failed:', err);
        statusText.textContent = `PRE-CACHE ERROR: ${err.message}`;
      } finally {
        btnPrecache.disabled = false;
        btnPrecache.textContent = '📥 Pre-Cache Disc';
        if (cacheProgressContainer) cacheProgressContainer.hidden = true;
        refreshCacheUI();
      }
    });
  }

  // =========================================================================
  // TrackMania Track Studio Engine & Canvas Builder
  // =========================================================================
  const TRACK_BLOCKS = [
    { id: 0, name: 'Start/Finish', short: 'START', color: '#22c55e', dot: '#22c55e' },
    { id: 1, name: 'Straight', short: 'STR', color: '#475569', dot: '#475569' },
    { id: 2, name: '90° Turn', short: 'T90', color: '#64748b', dot: '#64748b' },
    { id: 3, name: 'Hairpin', short: 'HAIR', color: '#94a3b8', dot: '#94a3b8' },
    { id: 4, name: 'Chicane', short: 'CHIC', color: '#cbd5e1', dot: '#cbd5e1' },
    { id: 5, name: 'Dirt Section', short: 'DIRT', color: '#b45309', dot: '#b45309' },
    { id: 6, name: 'Ice Bobsleigh', short: 'ICE', color: '#38bdf8', dot: '#38bdf8' },
    { id: 7, name: 'Jump Ramp', short: 'JUMP', color: '#f97316', dot: '#f97316' },
    { id: 8, name: 'Bridge Pass', short: 'BRID', color: '#8b5cf6', dot: '#8b5cf6' },
    { id: 9, name: 'Free-Wheel Red', short: 'FREE', color: '#ef4444', dot: '#ef4444' },
    { id: 10, name: 'Reactor Boost', short: 'BOOST', color: '#06b6d4', dot: '#06b6d4' },
    { id: 11, name: 'Checkpoint Gate', short: 'GATE', color: '#eab308', dot: '#eab308' },
  ];

  const studioCanvas = document.getElementById('studio-canvas');
  if (studioCanvas) {
    const ctx = studioCanvas.getContext('2d');
    const trackNameInput = document.getElementById('track-name-input');
    const statBlocks = document.getElementById('stat-blocks');
    const statCheckpoints = document.getElementById('stat-checkpoints');
    const statStart = document.getElementById('stat-start');
    const validationBadge = document.getElementById('studio-validation-badge');
    const paletteGrid = document.getElementById('palette-grid');
    const canvasCoords = document.getElementById('canvas-coords');
    const btnToolErase = document.getElementById('btn-tool-erase');
    const btnStudioClear = document.getElementById('btn-studio-clear');
    const btnCopyUrl = document.getElementById('btn-copy-url');
    const btnImportCode = document.getElementById('btn-import-code');
    const btnExportSvg = document.getElementById('btn-export-svg');
    const btnExportMcd = document.getElementById('btn-export-mcd');
    const btnExportBin = document.getElementById('btn-export-bin');
    const rotBtns = document.querySelectorAll('.rot-btn');
    const presetBtns = document.querySelectorAll('.preset-btn');

    const GRID_DIM = 20;
    const MAX_BLOCKS = 64;
    const grid = Array.from({ length: GRID_DIM }, () => Array(GRID_DIM).fill(null));
    let selectedBlockId = 0;
    let currentOrientation = 0;
    let isEraseMode = false;
    let hoverCell = null;
    let isMouseDown = false;

    function crc16(data) {
      let crc = 0xFFFF;
      for (let i = 0; i < data.length; i++) {
        crc ^= (data[i] << 8);
        for (let j = 0; j < 8; j++) {
          if ((crc & 0x8000) !== 0) {
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF;
          } else {
            crc = (crc << 1) & 0xFFFF;
          }
        }
      }
      return crc;
    }

    function serializeTrack() {
      const name = (trackNameInput ? trackNameInput.value : 'CUSTOM').trim().toUpperCase() || 'CUSTOM';
      const buf = new Uint8Array(256);
      buf[0] = 0x41; buf[1] = 0x52; buf[2] = 0x54; buf[3] = 0x52; // "ARTR"
      buf[4] = 1; // Version

      const encoder = new TextEncoder();
      const nameBytes = encoder.encode(name.substring(0, 16));
      buf.set(nameBytes, 5);

      const placed = getPlacedBlocks();
      const count = Math.min(placed.length, MAX_BLOCKS);
      buf[25] = count;

      let offset = 28;
      for (let i = 0; i < count; i++) {
        const b = placed[i];
        buf[offset] = b.x & 0xFF;
        buf[offset + 1] = b.y & 0xFF;
        buf[offset + 2] = (b.kind & 0x0F) | ((b.orientation & 0x03) << 4);
        offset += 3;
      }

      const crcBuf = new Uint8Array(254);
      crcBuf.set(buf.subarray(0, 26), 0);
      crcBuf.set(buf.subarray(28, 256), 26);
      const checksum = crc16(crcBuf);
      const dv = new DataView(buf.buffer);
      dv.setUint16(26, checksum, true);

      return buf;
    }

    function deserializeTrack(buf) {
      if (buf.length !== 256) throw new Error('Expected 256 bytes');
      if (buf[0] !== 0x41 || buf[1] !== 0x52 || buf[2] !== 0x54 || buf[3] !== 0x52) {
        throw new Error('Invalid track magic');
      }
      const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
      const storedCrc = dv.getUint16(26, true);
      const crcBuf = new Uint8Array(254);
      crcBuf.set(buf.subarray(0, 26), 0);
      crcBuf.set(buf.subarray(28, 256), 26);
      if (crc16(crcBuf) !== storedCrc) {
        throw new Error('CRC16 verification failed');
      }

      let endIdx = 21;
      for (let i = 5; i < 21; i++) {
        if (buf[i] === 0) { endIdx = i; break; }
      }
      const name = new TextDecoder('ascii').decode(buf.subarray(5, endIdx));
      const count = Math.min(buf[25], MAX_BLOCKS);

      clearGrid();
      if (trackNameInput) trackNameInput.value = name;

      let off = 28;
      for (let i = 0; i < count; i++) {
        const x = buf[off];
        const y = buf[off + 1];
        const kind = buf[off + 2] & 0x0F;
        const orientation = (buf[off + 2] >> 4) & 0x03;
        if (x < GRID_DIM && y < GRID_DIM) {
          grid[y][x] = { kind, orientation };
        }
        off += 3;
      }
      updateUI();
    }

    function toBase64(buf) {
      let bin = '';
      for (let i = 0; i < buf.length; i++) bin += String.fromCharCode(buf[i]);
      return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    }

    function fromBase64(str) {
      let clean = str.trim();
      if (clean.startsWith('#track=')) clean = clean.substring(7);
      clean = clean.replace(/-/g, '+').replace(/_/g, '/');
      while (clean.length % 4) clean += '=';
      const binary = atob(clean);
      const buf = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) buf[i] = binary.charCodeAt(i);
      return deserializeTrack(buf);
    }

    function getPlacedBlocks() {
      const placed = [];
      for (let y = 0; y < GRID_DIM; y++) {
        for (let x = 0; x < GRID_DIM; x++) {
          if (grid[y][x]) {
            placed.push({ x, y, kind: grid[y][x].kind, orientation: grid[y][x].orientation });
          }
        }
      }
      return placed;
    }

    function clearGrid() {
      for (let y = 0; y < GRID_DIM; y++) {
        for (let x = 0; x < GRID_DIM; x++) {
          grid[y][x] = null;
        }
      }
      updateUI();
    }

    function updateUI() {
      const placed = getPlacedBlocks();
      const count = placed.length;
      let startCount = 0;
      let checkCount = 0;

      for (const b of placed) {
        if (b.kind === 0) startCount++;
        if (b.kind === 11) checkCount++;
      }

      if (statBlocks) statBlocks.textContent = `${count} / ${MAX_BLOCKS}`;
      if (statCheckpoints) statCheckpoints.textContent = `${checkCount}`;
      if (statStart) {
        statStart.textContent = startCount === 1 ? 'READY (1)' : startCount === 0 ? 'NONE' : `${startCount} (ERROR)`;
        statStart.style.color = startCount === 1 ? '#10b981' : '#f87171';
      }

      if (validationBadge) {
        if (count < 3) {
          validationBadge.className = 'studio-badge invalid';
          validationBadge.textContent = '⚠️ NEEDS AT LEAST 3 BLOCKS';
        } else if (startCount === 0) {
          validationBadge.className = 'studio-badge invalid';
          validationBadge.textContent = '⚠️ NEEDS START/FINISH GRID';
        } else if (startCount > 1) {
          validationBadge.className = 'studio-badge invalid';
          validationBadge.textContent = '⚠️ MULTIPLE START GRIDS (ONLY 1 ALLOWED)';
        } else {
          validationBadge.className = 'studio-badge valid';
          validationBadge.textContent = '✅ VALID TRACK READY TO RACE';
        }
      }

      renderCanvas();
    }

    function renderCanvas() {
      const size = studioCanvas.width;
      const cell = size / GRID_DIM;

      ctx.fillStyle = '#090c12';
      ctx.fillRect(0, 0, size, size);

      // Draw Grid lines
      ctx.strokeStyle = '#1a2233';
      ctx.lineWidth = 1;
      for (let i = 0; i <= GRID_DIM; i++) {
        const p = i * cell;
        ctx.beginPath();
        ctx.moveTo(p, 0); ctx.lineTo(p, size);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(0, p); ctx.lineTo(size, p);
        ctx.stroke();
      }

      // Draw Placed Blocks
      for (let y = 0; y < GRID_DIM; y++) {
        for (let x = 0; x < GRID_DIM; x++) {
          const b = grid[y][x];
          if (b) {
            drawBlock(ctx, x, y, b.kind, b.orientation, cell, 1.0);
          }
        }
      }

      // Draw Hover Ghost
      if (hoverCell && hoverCell.x >= 0 && hoverCell.x < GRID_DIM && hoverCell.y >= 0 && hoverCell.y < GRID_DIM) {
        if (isEraseMode) {
          ctx.fillStyle = 'rgba(239, 68, 68, 0.4)';
          ctx.fillRect(hoverCell.x * cell + 1, hoverCell.y * cell + 1, cell - 2, cell - 2);
        } else {
          drawBlock(ctx, hoverCell.x, hoverCell.y, selectedBlockId, currentOrientation, cell, 0.5);
        }
      }
    }

    function drawBlock(c, x, y, kind, rot, cell, alpha) {
      c.save();
      c.globalAlpha = alpha;
      const bx = x * cell;
      const by = y * cell;
      const cx = bx + cell / 2;
      const cy = by + cell / 2;
      const def = TRACK_BLOCKS[kind] || TRACK_BLOCKS[1];

      c.translate(cx, cy);
      c.rotate((rot * 90 * Math.PI) / 180);
      c.translate(-cx, -cy);

      // Block background
      c.fillStyle = def.color;
      c.fillRect(bx + 2, by + 2, cell - 4, cell - 4);
      c.strokeStyle = '#000';
      c.lineWidth = 1.5;
      c.strokeRect(bx + 2, by + 2, cell - 4, cell - 4);

      // Internal decorative details
      if (kind === 0) { // Start/Finish grid checks
        c.fillStyle = '#000';
        for (let cyc = 0; cyc < 4; cyc++) {
          for (let cxc = 0; cxc < 4; cxc++) {
            if ((cxc + cyc) % 2 === 0) {
              c.fillRect(bx + 2 + cxc * ((cell - 4) / 4), by + 2 + cyc * ((cell - 4) / 4), (cell - 4) / 4, (cell - 4) / 4);
            }
          }
        }
      } else if (kind === 10) { // Reactor boost chevron
        c.fillStyle = '#ffffff';
        c.beginPath();
        c.moveTo(cx, by + 6);
        c.lineTo(bx + 6, cy + 6);
        c.lineTo(bx + cell - 6, cy + 6);
        c.closePath();
        c.fill();
      }

      // Direction indicator arrow
      c.fillStyle = '#ffffff';
      c.beginPath();
      c.moveTo(cx, by + 3);
      c.lineTo(cx - 3, by + 8);
      c.lineTo(cx + 3, by + 8);
      c.closePath();
      c.fill();

      // Block label text
      c.fillStyle = '#ffffff';
      c.font = `bold ${Math.floor(cell * 0.28)}px system-ui, sans-serif`;
      c.textAlign = 'center';
      c.textBaseline = 'middle';
      c.shadowColor = '#000';
      c.shadowBlur = 3;
      c.fillText(def.short, cx, cy + 4);

      c.restore();
    }

    // Populate Palette Grid
    if (paletteGrid) {
      paletteGrid.innerHTML = '';
      TRACK_BLOCKS.forEach((blk, idx) => {
        const btn = document.createElement('button');
        btn.className = `palette-btn ${idx === selectedBlockId ? 'active' : ''}`;
        btn.dataset.id = blk.id;
        btn.innerHTML = `<span class="blk-dot" style="background: ${blk.dot}"></span><span>${blk.name}</span>`;
        btn.addEventListener('click', () => {
          selectedBlockId = blk.id;
          isEraseMode = false;
          if (btnToolErase) btnToolErase.classList.remove('active');
          document.querySelectorAll('.palette-btn').forEach(b => b.classList.remove('active'));
          btn.classList.add('active');
          renderCanvas();
        });
        paletteGrid.appendChild(btn);
      });
    }

    // Rotation controls
    rotBtns.forEach(btn => {
      btn.addEventListener('click', () => {
        currentOrientation = parseInt(btn.dataset.rot, 10);
        rotBtns.forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        renderCanvas();
      });
    });

    function setRotation(rot) {
      currentOrientation = rot % 4;
      rotBtns.forEach(b => {
        b.classList.toggle('active', parseInt(b.dataset.rot, 10) === currentOrientation);
      });
      renderCanvas();
    }

    window.addEventListener('keydown', (e) => {
      if (e.target && e.target.tagName === 'INPUT') return;
      if (e.key === 'r' || e.key === 'R') {
        setRotation(currentOrientation + 1);
      }
    });

    if (btnToolErase) {
      btnToolErase.addEventListener('click', () => {
        isEraseMode = !isEraseMode;
        btnToolErase.classList.toggle('active', isEraseMode);
        renderCanvas();
      });
    }

    if (btnStudioClear) {
      btnStudioClear.addEventListener('click', () => {
        if (confirm('Clear all blocks on the grid?')) {
          clearGrid();
        }
      });
    }

    // Canvas Mouse Interaction
    function getCellFromMouse(e) {
      const rect = studioCanvas.getBoundingClientRect();
      const scaleX = studioCanvas.width / rect.width;
      const scaleY = studioCanvas.height / rect.height;
      const x = Math.floor(((e.clientX - rect.left) * scaleX) / (studioCanvas.width / GRID_DIM));
      const y = Math.floor(((e.clientY - rect.top) * scaleY) / (studioCanvas.height / GRID_DIM));
      return { x, y };
    }

    studioCanvas.addEventListener('mousemove', (e) => {
      const cell = getCellFromMouse(e);
      hoverCell = cell;
      if (canvasCoords) canvasCoords.textContent = `X: ${cell.x}, Y: ${cell.y}`;
      if (isMouseDown) {
        applyTool(cell, e.buttons === 2 || isEraseMode);
      }
      renderCanvas();
    });

    studioCanvas.addEventListener('mouseleave', () => {
      hoverCell = null;
      isMouseDown = false;
      if (canvasCoords) canvasCoords.textContent = 'X: --, Y: --';
      renderCanvas();
    });

    studioCanvas.addEventListener('mousedown', (e) => {
      isMouseDown = true;
      const cell = getCellFromMouse(e);
      applyTool(cell, e.button === 2 || isEraseMode);
    });

    window.addEventListener('mouseup', () => {
      isMouseDown = false;
    });

    studioCanvas.addEventListener('contextmenu', (e) => {
      e.preventDefault();
    });

    function applyTool(cell, erase) {
      if (cell.x < 0 || cell.x >= GRID_DIM || cell.y < 0 || cell.y >= GRID_DIM) return;
      if (erase) {
        grid[cell.y][cell.x] = null;
      } else {
        const placed = getPlacedBlocks();
        if (placed.length >= MAX_BLOCKS && !grid[cell.y][cell.x]) {
          alert('Maximum of 64 blocks placed!');
          return;
        }
        grid[cell.y][cell.x] = { kind: selectedBlockId, orientation: currentOrientation };
      }
      updateUI();
    }

    // Templates
    presetBtns.forEach(btn => {
      btn.addEventListener('click', () => {
        const preset = btn.dataset.preset;
        clearGrid();
        if (preset === 'speed_ring') {
          if (trackNameInput) trackNameInput.value = 'SPEED RING';
          grid[4][6] = { kind: 0, orientation: 1 }; // Start
          grid[4][7] = { kind: 1, orientation: 1 };
          grid[4][8] = { kind: 10, orientation: 1 }; // Reactor boost
          grid[4][9] = { kind: 1, orientation: 1 };
          grid[4][10] = { kind: 2, orientation: 1 }; // Turn 90
          grid[5][10] = { kind: 1, orientation: 2 };
          grid[6][10] = { kind: 11, orientation: 2 }; // Checkpoint
          grid[7][10] = { kind: 1, orientation: 2 };
          grid[8][10] = { kind: 2, orientation: 2 };
          grid[8][9] = { kind: 1, orientation: 3 };
          grid[8][8] = { kind: 10, orientation: 3 };
          grid[8][7] = { kind: 1, orientation: 3 };
          grid[8][6] = { kind: 2, orientation: 3 };
          grid[7][6] = { kind: 1, orientation: 0 };
          grid[6][6] = { kind: 11, orientation: 0 };
          grid[5][6] = { kind: 2, orientation: 0 };
        } else if (preset === 'stunt_arena') {
          if (trackNameInput) trackNameInput.value = 'STUNT ARENA';
          grid[5][5] = { kind: 0, orientation: 1 };
          grid[5][6] = { kind: 10, orientation: 1 }; // Boost
          grid[5][7] = { kind: 7, orientation: 1 };  // Jump ramp
          grid[5][8] = { kind: 11, orientation: 1 }; // Checkpoint
          grid[5][9] = { kind: 8, orientation: 1 };  // Bridge
          grid[5][10] = { kind: 2, orientation: 1 };
          grid[6][10] = { kind: 1, orientation: 2 };
          grid[7][10] = { kind: 9, orientation: 2 }; // Free-wheel red
          grid[8][10] = { kind: 2, orientation: 2 };
          grid[8][9] = { kind: 1, orientation: 3 };
          grid[8][8] = { kind: 7, orientation: 3 };
          grid[8][7] = { kind: 1, orientation: 3 };
          grid[8][6] = { kind: 11, orientation: 3 };
          grid[8][5] = { kind: 2, orientation: 3 };
          grid[7][5] = { kind: 5, orientation: 0 };  // Dirt
          grid[6][5] = { kind: 1, orientation: 0 };
        } else if (preset === 'bobsleigh_trial') {
          if (trackNameInput) trackNameInput.value = 'BOBSLEIGH TRIAL';
          grid[4][4] = { kind: 0, orientation: 1 };
          grid[4][5] = { kind: 6, orientation: 1 };  // Ice
          grid[4][6] = { kind: 6, orientation: 1 };
          grid[4][7] = { kind: 2, orientation: 1 };
          grid[5][7] = { kind: 6, orientation: 2 };
          grid[6][7] = { kind: 11, orientation: 2 };
          grid[7][7] = { kind: 3, orientation: 2 };  // Hairpin
          grid[7][6] = { kind: 6, orientation: 3 };
          grid[7][5] = { kind: 4, orientation: 3 };  // Chicane
          grid[7][4] = { kind: 2, orientation: 3 };
          grid[6][4] = { kind: 11, orientation: 0 };
          grid[5][4] = { kind: 1, orientation: 0 };
        }
        updateUI();
      });
    });

    // Copy Share URL
    if (btnCopyUrl) {
      btnCopyUrl.addEventListener('click', () => {
        try {
          const raw = serializeTrack();
          const b64 = toBase64(raw);
          const url = `${window.location.origin}${window.location.pathname}#track=${b64}`;
          window.location.hash = `track=${b64}`;
          navigator.clipboard.writeText(url).then(() => {
            btnCopyUrl.textContent = '✅ Copied URL!';
            setTimeout(() => { btnCopyUrl.textContent = '🔗 Copy Share URL'; }, 2000);
          }).catch(() => {
            prompt('Copy your shareable track URL:', url);
          });
        } catch (err) {
          alert('Error exporting track: ' + err.message);
        }
      });
    }

    // Import Code
    if (btnImportCode) {
      btnImportCode.addEventListener('click', () => {
        const input = prompt('Paste shareable Track Code (#track=... or Base64):');
        if (input) {
          try {
            fromBase64(input);
            alert('Track loaded successfully!');
          } catch (err) {
            alert('Failed to import track: ' + err.message);
          }
        }
      });
    }

    // Export SVG
    if (btnExportSvg) {
      btnExportSvg.addEventListener('click', () => {
        const placed = getPlacedBlocks();
        const name = (trackNameInput ? trackNameInput.value : 'CUSTOM').trim();
        const cell = 30;
        const total = cell * GRID_DIM;
        let svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${total} ${total}" width="${total}" height="${total}">\n`;
        svg += `  <rect width="${total}" height="${total}" fill="#0d1117"/>\n`;

        for (const b of placed) {
          const bx = b.x * cell;
          const by = b.y * cell;
          const cx = bx + cell / 2;
          const cy = by + cell / 2;
          const def = TRACK_BLOCKS[b.kind] || TRACK_BLOCKS[1];
          const rot = b.orientation * 90;
          svg += `  <g transform="rotate(${rot} ${cx} ${cy})">\n`;
          svg += `    <rect x="${bx + 2}" y="${by + 2}" width="${cell - 4}" height="${cell - 4}" rx="3" fill="${def.color}" stroke="#000" stroke-width="1"/>\n`;
          svg += `    <polygon points="${cx},${by + 3} ${cx - 3},${by + 7} ${cx + 3},${by + 7}" fill="#ffffff"/>\n`;
          svg += `    <text x="${cx}" y="${cy + 3}" fill="#ffffff" font-family="sans-serif" font-size="8" font-weight="bold" text-anchor="middle">${def.short}</text>\n`;
          svg += `  </g>\n`;
        }
        svg += `  <text x="10" y="${total - 10}" fill="#94a3b8" font-family="monospace" font-size="10">${name} • ARDURACER PSX</text>\n`;
        svg += `</svg>`;

        const blob = new Blob([svg], { type: 'image/svg+xml' });
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `${name.replace(/\s+/g, '_').toLowerCase()}.svg`;
        a.click();
      });
    }

    // Export .bin
    if (btnExportBin) {
      btnExportBin.addEventListener('click', () => {
        const raw = serializeTrack();
        const name = (trackNameInput ? trackNameInput.value : 'CUSTOM').trim();
        const blob = new Blob([raw], { type: 'application/octet-stream' });
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `${name.replace(/\s+/g, '_').toLowerCase()}.bin`;
        a.click();
      });
    }

    // Export .mcd PS1 Memory Card
    if (btnExportMcd) {
      btnExportMcd.addEventListener('click', () => {
        const trackRaw = serializeTrack();
        const mcd = new Uint8Array(128 * 1024); // 128 KB
        const payload = new Uint8Array(1412); // SAVE_VERSION = 7

        // "BAS-ARDURACER-01"
        const magic = new TextEncoder().encode('BAS-ARDURACER-01');
        payload.set(magic, 0);
        const dvPayload = new DataView(payload.buffer);
        dvPayload.setUint16(16, 7, true); // Version 7
        payload[18] = 31; // Unlocked stages
        payload[177] = 0; // Texture format
        payload[178] = 0; // Framerate
        payload[179] = 1; // CD Audio
        payload[180] = 8; // SFX Vol
        payload[181] = 8; // Music Vol
        payload[182] = 0; // Profile

        // Custom Track Slot 0 at offset 386
        payload.set(trackRaw, 386);

        // Compute Save CRC16 over 0..1410
        const saveCrc = crc16(payload.subarray(0, 1410));
        dvPayload.setUint16(1410, saveCrc, true);

        // Put payload in block 0
        mcd.set(payload, 0);

        // Format Directory Frame (block 15)
        const dirOffset = 15 * 8192;
        mcd[dirOffset] = 0x51; // In use, 1 block
        const titleBytes = new TextEncoder().encode('BA-ARDURACER-DATA');
        mcd.set(titleBytes, dirOffset + 12);

        const name = (trackNameInput ? trackNameInput.value : 'CUSTOM').trim();
        const blob = new Blob([mcd], { type: 'application/octet-stream' });
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `arduracer_${name.replace(/\s+/g, '_').toLowerCase()}.mcd`;
        a.click();
      });
    }

    // Auto-load hash if #track=...
    if (window.location.hash && window.location.hash.includes('track=')) {
      try {
        fromBase64(window.location.hash);
        // Switch to Studio tab
        const studioBtn = document.querySelector('.tab-btn[data-tab="tab-studio"]');
        if (studioBtn) studioBtn.click();
      } catch (err) {
        console.warn('Could not auto-load track from hash:', err);
      }
    } else {
      // Load default speed ring template initially
      const presetSpeed = document.querySelector('.preset-btn[data-preset="speed_ring"]');
      if (presetSpeed) presetSpeed.click();
    }
  }
});

