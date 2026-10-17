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

  async function updateSizesFromManifest() {
    if (!window.romCache || typeof window.romCache.getManifest !== 'function') return;
    const manifest = await window.romCache.getManifest();
    if (!manifest || !manifest.files) return;

    const files = manifest.files;
    const binSize = files['arduracer.bin'] ? RomCacheManager.formatBytes(files['arduracer.bin'].size) : null;
    const isoSize = files['arduracer.iso'] ? RomCacheManager.formatBytes(files['arduracer.iso'].size) : null;
    const exeSize = files['arduracer.exe'] ? RomCacheManager.formatBytes(files['arduracer.exe'].size) : null;

    if (discSelect) {
      Array.from(discSelect.options).forEach((opt) => {
        if (opt.value.includes('arduracer.cue') && binSize) {
          opt.textContent = `💿 Master CD-ROM (Full Audio & FMV - CUE/BIN, ${binSize})`;
        } else if (opt.value.includes('arduracer.bin') && binSize) {
          opt.textContent = `💿 Raw Bin Image (Track 1 Data + CDDA - ${binSize})`;
        } else if (opt.value.includes('arduracer.iso') && isoSize) {
          opt.textContent = `📀 Compact Disc ISO (Data Only Track - ${isoSize})`;
        } else if (opt.value.includes('arduracer.exe') && exeSize) {
          opt.textContent = `🚀 Fast Boot (Standalone PSX-EXE - ${exeSize})`;
        }
      });
    }

    const dlExe = document.querySelector('a[href$="arduracer.exe"]');
    if (dlExe && exeSize) {
      dlExe.textContent = `⬇ Download arduracer.exe (${exeSize})`;
      const desc = dlExe.closest('.download-card')?.querySelector('p');
      if (desc) desc.textContent = `MIPS R3000 bare-metal executable (${exeSize}). Fast boot via Unirom, Caetla, or emulator rapid testing.`;
    }

    const dlIso = document.querySelector('a[href$="arduracer.iso"]');
    if (dlIso && isoSize) {
      dlIso.textContent = `⬇ Download arduracer.iso (${isoSize})`;
      const desc = dlIso.closest('.download-card')?.querySelector('p');
      if (desc) desc.textContent = `Standard 2048-byte Mode 1 ISO filesystem (${isoSize}). Compact disc image compatible with universal optical disc loaders.`;
    }

    const dlBin = document.querySelector('a[href$="arduracer.bin"]');
    if (dlBin && binSize) {
      dlBin.textContent = `⬇ Download arduracer.bin (${binSize})`;
      const desc = dlBin.closest('.download-card')?.querySelector('p');
      if (desc) desc.textContent = `Complete Red Book CD image (${binSize}) including 6 CD-DA Eurobeat soundtrack tracks and 15fps MDEC intro cinema.`;
    }
  }

  if (window.romCache) {
    updateSizesFromManifest();

    window.romCache.init().then(() => {
      refreshCacheUI();
      updateSizesFromManifest();
    });

    window.romCache.subscribe((event, data) => {
      if (event === 'new-image') {
        statusLed.style.background = '#f59e0b';
        statusText.textContent = 'NEW DISC IMAGE DETECTED • CACHE FLUSHED';
        refreshCacheUI();
        updateSizesFromManifest();
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
});
