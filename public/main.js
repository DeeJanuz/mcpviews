// @ts-nocheck
/* MCPViews — Tauri WebView client
 * Multi-session tab bar with Tauri IPC events.
 */

(function () {
  'use strict';

  let activeSessionId = null;

  /** @type {Map<string, {toolName: string, contentType: string, data: unknown, meta: Record<string, unknown>, toolArgs: Record<string, unknown>, reviewRequired: boolean, timestamp: number}>} */
  const sessions = new Map();

  // DOM refs
  const contentArea = document.getElementById('content-area');
  const mainTitle = document.getElementById('main-title');
  const connectionDot = document.getElementById('connection-dot');
  const connectionText = document.getElementById('connection-text');
  const tabBar = document.getElementById('tab-bar');

  /** @type {Map<string, HTMLElement>} Cached content containers per session */
  const contentCache = new Map();

  /** @type {Map<string, {deadline: number, intervalId: number}>} Countdown timers per review session */
  const countdownTimers = new Map();

  // --- Heartbeat ---
  let heartbeatInterval = null;
  let lastActivity = Date.now();

  function startHeartbeat(sessionId) {
    stopHeartbeat();
    lastActivity = Date.now();

    var onActivity = function () {
      lastActivity = Date.now();
      if (activeSessionId) resetCountdown(activeSessionId);
    };
    contentArea.addEventListener('click', onActivity);
    contentArea.addEventListener('scroll', onActivity);
    contentArea.addEventListener('keydown', onActivity);
    contentArea.addEventListener('input', onActivity);

    // Store cleanup ref
    contentArea._heartbeatCleanup = function () {
      contentArea.removeEventListener('click', onActivity);
      contentArea.removeEventListener('scroll', onActivity);
      contentArea.removeEventListener('keydown', onActivity);
      contentArea.removeEventListener('input', onActivity);
    };

    heartbeatInterval = window.setInterval(function () {
      // Only send if user was active in last 60s
      if (Date.now() - lastActivity < 60000) {
        fetch('/api/heartbeat', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ session_id: sessionId }),
        }).catch(function () {});
      }
    }, 30000);
  }

  function stopHeartbeat() {
    if (heartbeatInterval) {
      clearInterval(heartbeatInterval);
      heartbeatInterval = null;
    }
    if (contentArea._heartbeatCleanup) {
      contentArea._heartbeatCleanup();
      contentArea._heartbeatCleanup = null;
    }
  }

  // --- Tab Bar ---

  function renderTabBar() {
    tabBar.innerHTML = '';
    if (sessions.size === 0) {
      tabBar.style.display = 'none';
      return;
    }
    tabBar.style.display = 'flex';

    sessions.forEach(function (session, sessionId) {
      var tab = document.createElement('div');
      tab.className = 'tab' + (sessionId === activeSessionId ? ' active' : '');
      tab.setAttribute('data-session-id', sessionId);

      if (session.reviewRequired) {
        var dot = document.createElement('span');
        dot.className = 'review-dot';
        tab.appendChild(dot);
      }

      var label = getTabLabel(session);
      var nameSpan = document.createElement('span');
      nameSpan.className = 'tab-name';
      nameSpan.textContent = label;
      nameSpan.title = label;
      tab.appendChild(nameSpan);

      if (session.reviewRequired && countdownTimers.has(sessionId)) {
        var timerSpan = document.createElement('span');
        timerSpan.className = 'tab-timer';
        tab.appendChild(timerSpan);
        // Will be updated by updateCountdownDisplay on next tick
      }

      var closeBtn = document.createElement('span');
      closeBtn.className = 'close-btn';
      closeBtn.textContent = '\u00d7';
      closeBtn.title = 'Close tab';
      closeBtn.addEventListener('click', function (e) {
        e.stopPropagation();
        closeTab(sessionId);
      });
      tab.appendChild(closeBtn);

      tab.addEventListener('click', function () {
        selectSession(sessionId);
      });

      tabBar.appendChild(tab);
    });

    // Update countdown displays after DOM is built
    countdownTimers.forEach(function (_, sid) {
      updateCountdownDisplay(sid);
    });
  }

  function removeSession(sessionId) {
    // Close any open drawers when session is removed
    if (window.__companionUtils && window.__companionUtils.closeAllDrawers) {
      window.__companionUtils.closeAllDrawers();
    }
    stopHeartbeat();
    stopCountdown(sessionId);
    sessions.delete(sessionId);

    // Remove cached content container
    var cached = contentCache.get(sessionId);
    if (cached && cached.parentNode) {
      cached.parentNode.removeChild(cached);
    }
    contentCache.delete(sessionId);

    if (sessionId === activeSessionId) {
      activeSessionId = null;
      var keys = Array.from(sessions.keys());
      if (keys.length > 0) {
        selectSession(keys[keys.length - 1]);
      } else {
        renderEmpty();
        renderTabBar();
      }
    } else {
      renderTabBar();
    }
  }

  function closeTab(sessionId) {
    // Dismiss session via Tauri IPC (handles review dismissal too)
    if (window.__TAURI__) {
      window.__TAURI__.core.invoke('dismiss_session', {
        sessionId: sessionId,
      }).catch(function (err) {
        console.error('Failed to dismiss session:', err);
      });
    }

    removeSession(sessionId);
  }

  // --- Countdown Timer ---

  function startCountdown(sessionId, timeoutSecs) {
    stopCountdown(sessionId);
    var deadline = Date.now() + (timeoutSecs * 1000);
    var intervalId = window.setInterval(function () {
      updateCountdownDisplay(sessionId);
    }, 1000);
    countdownTimers.set(sessionId, { deadline: deadline, intervalId: intervalId });
    updateCountdownDisplay(sessionId);
  }

  function resetCountdown(sessionId) {
    var timer = countdownTimers.get(sessionId);
    if (!timer) return;
    var session = sessions.get(sessionId);
    var timeoutSecs = (session && session.timeoutSecs) || 120;
    timer.deadline = Date.now() + (timeoutSecs * 1000);
    updateCountdownDisplay(sessionId);
  }

  function stopCountdown(sessionId) {
    var timer = countdownTimers.get(sessionId);
    if (timer) {
      clearInterval(timer.intervalId);
      countdownTimers.delete(sessionId);
    }
  }

  function updateCountdownDisplay(sessionId) {
    var timer = countdownTimers.get(sessionId);
    var timerEl = tabBar.querySelector('.tab[data-session-id="' + sessionId + '"] .tab-timer');
    if (!timer || !timerEl) return;
    var remaining = Math.max(0, Math.ceil((timer.deadline - Date.now()) / 1000));
    var mins = Math.floor(remaining / 60);
    var secs = remaining % 60;
    timerEl.textContent = mins + ':' + (secs < 10 ? '0' : '') + secs;
    if (remaining <= 30) {
      timerEl.classList.add('urgent');
    } else {
      timerEl.classList.remove('urgent');
    }
  }

  // --- Tauri IPC ---

  async function initTauri() {
    // Wait for Tauri APIs to be available
    if (!window.__TAURI__) {
      // In dev mode without Tauri, fall back to polling localhost:4200
      console.log('Tauri API not available, running in standalone browser mode');
      connectionDot.classList.remove('connected');
      connectionText.textContent = 'Browser Mode';
      return;
    }

    const { listen } = window.__TAURI__.event;
    const { invoke } = window.__TAURI__.core;

    // Listen for push events from Rust backend
    await listen('push_preview', function (event) {
      const session = event.payload;
      handlePush(session);
    });

    // Load plugin renderers before rendering any sessions
    await loadPluginRenderers();

    // Load any existing sessions on startup (after renderers are ready)
    try {
      const existingSessions = await invoke('get_sessions');
      if (existingSessions && existingSessions.length > 0) {
        existingSessions.forEach(function (session) {
          handlePush(session);
        });
      }
    } catch (e) {
      console.error('Failed to load existing sessions:', e);
    }

    // Populate invocation registry
    if (window.__companionUtils && window.__companionUtils.populateRendererRegistry) {
      window.__companionUtils.populateRendererRegistry();
    }

    // Reload renderers when a plugin is installed
    await listen('reload_renderers', function () {
      loadPluginRenderers();
      // Populate invocation registry
      if (window.__companionUtils && window.__companionUtils.populateRendererRegistry) {
        window.__companionUtils.populateRendererRegistry();
      }
    });

    connectionDot.classList.add('connected');
    connectionText.textContent = 'Ready';
  }

  async function loadPluginRenderers() {
    if (!window.__TAURI__) return;
    try {
      var renderers = await window.__TAURI__.core.invoke('get_plugin_renderers');
      var loadPromises = [];
      renderers.forEach(function (renderer) {
        // Check if already loaded
        var existing = document.querySelector('script[data-plugin-renderer="' + renderer.plugin_name + '/' + renderer.file_name + '"]');
        if (existing) return;

        var promise = new Promise(function (resolve) {
          var script = document.createElement('script');
          script.src = renderer.url;
          script.setAttribute('data-plugin-renderer', renderer.plugin_name + '/' + renderer.file_name);
          script.onload = resolve;
          script.onerror = function () {
            console.error('[mcpviews] Failed to load plugin renderer:', renderer.url);
            resolve(); // resolve anyway so other renderers aren't blocked
          };
          document.head.appendChild(script);
        });
        loadPromises.push(promise);
      });
      await Promise.all(loadPromises);
    } catch (e) {
      console.error('[mcpviews] Failed to load plugin renderers:', e);
    }
  }

  // --- Message Handling ---

  function handlePush(session) {
    sessions.set(session.sessionId, {
      toolName: session.toolName,
      contentType: session.contentType,
      data: session.data,
      meta: session.meta || {},
      toolArgs: session.toolArgs || {},
      reviewRequired: session.reviewRequired,
      timeoutSecs: session.timeoutSecs || null,
      timestamp: session.createdAt || Date.now(),
    });

    // Start countdown timer for review sessions
    if (session.reviewRequired && session.timeoutSecs) {
      startCountdown(session.sessionId, session.timeoutSecs);
    }

    // Always auto-focus the new tab
    selectSession(session.sessionId);
  }

  function getTabLabel(session) {
    // Try to extract a meaningful label from the data
    if (session.data && typeof session.data === 'object') {
      if (session.data.title && typeof session.data.title === 'string') {
        return session.data.title;
      }
      if (session.data.name && typeof session.data.name === 'string') {
        return session.data.name;
      }
    }
    // Fall back to toolArgs title if present
    if (session.toolArgs && session.toolArgs.title && typeof session.toolArgs.title === 'string') {
      return session.toolArgs.title;
    }
    // Fall back to tool name
    return session.toolName;
  }

  // --- Rendering ---

  function selectSession(sessionId) {
    activeSessionId = sessionId;
    var session = sessions.get(sessionId);
    if (session && session.reviewRequired) {
      startHeartbeat(sessionId);
    } else {
      stopHeartbeat();
    }
    renderTabBar();
    renderContent(sessionId);
  }

  function renderContent(sessionId) {
    const session = sessions.get(sessionId);
    if (!session) {
      renderEmpty();
      return;
    }

    mainTitle.textContent = session.toolName + ' \u2014 ' + session.contentType;

    // Hide all cached containers
    contentCache.forEach(function (container) {
      container.style.display = 'none';
    });

    // Hide empty state if present
    var emptyState = contentArea.querySelector('.empty-state');
    if (emptyState) {
      emptyState.style.display = 'none';
    }

    // Check if we already have a cached container for this session
    var cached = contentCache.get(sessionId);
    if (cached) {
      cached.style.display = 'block';
      return;
    }

    // Create new container and render
    var container = document.createElement('div');
    container.className = 'session-content';
    container.setAttribute('data-session-id', sessionId);
    contentArea.appendChild(container);
    contentCache.set(sessionId, container);

    const renderer = getRenderer(session.contentType);
    renderer(container, session.data, session.meta, session.toolArgs || {}, session.reviewRequired, function (decision) {
      onDecision(sessionId, decision);
    });
  }

  function renderEmpty() {
    mainTitle.textContent = 'MCPViews';
    // Hide all cached containers
    contentCache.forEach(function (container) {
      container.style.display = 'none';
    });
    // Show empty state if no sessions
    var emptyState = contentArea.querySelector('.empty-state');
    if (!emptyState) {
      emptyState = document.createElement('div');
      emptyState.className = 'empty-state';
      emptyState.textContent = 'Waiting for preview data...';
      contentArea.appendChild(emptyState);
    }
    emptyState.style.display = '';
  }

  function getRenderer(contentType) {
    var renderers = window.__renderers || {};
    if (contentType && typeof renderers[contentType] === 'function') {
      return renderers[contentType];
    }
    return function renderError(container) {
      container.innerHTML = '<div style="color:#ef4444;padding:32px;text-align:center;">' +
        '<h3>No renderer for content type: ' + (contentType || 'unknown') + '</h3>' +
        '<p style="color:#737373;">This tool needs a renderer added to the UI.</p></div>';
    };
  }

  // --- Decision ---

  function onDecision(sessionId, decision) {
    // Build the decision payload for Tauri IPC
    var decisionStr = '';
    var operationDecisions = null;
    var comments = null;
    var modifications = null;
    var additions = null;

    if (typeof decision === 'string') {
      decisionStr = decision;
    } else if (typeof decision === 'object') {
      if (decision.type === 'review_decision') {
        decisionStr = decision.decision;
      } else if (decision.type === 'operation_decisions') {
        decisionStr = 'partial';
        operationDecisions = decision.decisions;
        if (decision.comments) comments = decision.comments;
        if (decision.modifications) modifications = decision.modifications;
        if (decision.additions) additions = decision.additions;
      } else {
        decisionStr = 'partial';
        operationDecisions = decision;
      }
    }

    // Submit via Tauri IPC
    if (window.__TAURI__) {
      window.__TAURI__.core.invoke('submit_decision', {
        sessionId: sessionId,
        decision: decisionStr,
        operationDecisions: operationDecisions,
        comments: comments,
        modifications: modifications,
        additions: additions,
      }).catch(function (err) {
        console.error('Failed to submit decision:', err);
      });
    }

    removeSession(sessionId);
  }

  // --- Global citation click handler ---

  document.addEventListener('click', function (e) {
    var citeEl = e.target.closest('[data-cite-type]');
    if (!citeEl) return;

    var type = citeEl.getAttribute('data-cite-type');
    var index = parseInt(citeEl.getAttribute('data-cite-index') || '0', 10);

    var session = activeSessionId ? sessions.get(activeSessionId) : null;
    if (!session) return;

    var data = session.data;
    var citationData = null;

    if (Array.isArray(data)) {
      citationData = data[index] || data[index - 1] || null;
    } else if (data && data.results && Array.isArray(data.results)) {
      citationData = data.results[index] || data.results[index - 1] || null;
    } else if (data && typeof data === 'object') {
      if (data.entries && Array.isArray(data.entries)) {
        citationData = data.entries[index] || data.entries[index - 1] || null;
      } else {
        citationData = data;
      }
    }

    if (citationData && window.__companionUtils && window.__companionUtils.openCitationPanel) {
      window.__companionUtils.openCitationPanel(type, citationData);
    }
  });

  // --- Global mcpview:// invocation click handler ---

  document.addEventListener('click', function (e) {
    var el = e.target.closest('[data-invoke-renderer]');
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();

    var rendererName = el.getAttribute('data-invoke-renderer');
    var paramsStr = el.getAttribute('data-invoke-params');
    var params = {};
    try { params = JSON.parse(paramsStr || '{}'); } catch (err) {}

    // Look up display mode from registry, fallback to 'drawer'
    var registry = window.__rendererRegistry || {};
    var meta = registry[rendererName];
    var displayMode = (meta && meta.display_mode) || 'drawer';

    if (window.__companionUtils && window.__companionUtils.invokeRenderer) {
      window.__companionUtils.invokeRenderer(rendererName, params, displayMode);
    }
  });

  // --- Escape key closes topmost drawer ---

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && window.__companionUtils && window.__companionUtils.closeDrawer) {
      window.__companionUtils.closeDrawer();
    }
  });

  // --- Init ---

  renderEmpty();
  initTauri();
})();
