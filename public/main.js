// @ts-nocheck
/* MCP Mux — Tauri WebView client
 * Replaces companion's WebSocket-based app.js with Tauri IPC events.
 */

(function () {
  'use strict';

  let activeSessionId = null;

  /** @type {Map<string, {toolName: string, contentType: string, data: unknown, meta: Record<string, unknown>, toolArgs: Record<string, unknown>, reviewRequired: boolean, timestamp: number}>} */
  const sessions = new Map();

  /** @type {string[]} */
  const queuedSessionIds = [];

  // DOM refs
  const contentArea = document.getElementById('content-area');
  const mainTitle = document.getElementById('main-title');
  const connectionDot = document.getElementById('connection-dot');
  const connectionText = document.getElementById('connection-text');

  // --- Heartbeat ---
  let heartbeatInterval = null;
  let lastActivity = Date.now();

  function startHeartbeat(sessionId) {
    stopHeartbeat();
    lastActivity = Date.now();

    var onActivity = function () { lastActivity = Date.now(); };
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

    // Load any existing sessions on startup
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

    // Load plugin renderers after initial sessions are loaded
    await loadPluginRenderers();

    // Reload renderers when a plugin is installed
    await listen('reload_renderers', function () {
      loadPluginRenderers();
    });

    connectionDot.classList.add('connected');
    connectionText.textContent = 'Ready';
  }

  async function loadPluginRenderers() {
    if (!window.__TAURI__) return;
    try {
      var renderers = await window.__TAURI__.core.invoke('get_plugin_renderers');
      renderers.forEach(function (renderer) {
        // Check if already loaded
        var existing = document.querySelector('script[data-plugin-renderer="' + renderer.plugin_name + '/' + renderer.file_name + '"]');
        if (existing) return;

        var script = document.createElement('script');
        script.src = renderer.url;
        script.setAttribute('data-plugin-renderer', renderer.plugin_name + '/' + renderer.file_name);
        script.onerror = function () {
          console.error('[mcp-mux] Failed to load plugin renderer:', renderer.url);
        };
        document.head.appendChild(script);
      });
    } catch (e) {
      console.error('[mcp-mux] Failed to load plugin renderers:', e);
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
      timestamp: session.createdAt || Date.now(),
    });

    // If a review is active, queue the new session instead of switching
    var activeSession = activeSessionId ? sessions.get(activeSessionId) : null;
    if (activeSession && activeSession.reviewRequired && session.sessionId !== activeSessionId) {
      queuedSessionIds.push(session.sessionId);
      updateQueueBadge();
    } else {
      selectSession(session.sessionId);
    }
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
    renderContent(sessionId);
  }

  function renderContent(sessionId) {
    const session = sessions.get(sessionId);
    if (!session) {
      renderEmpty();
      return;
    }

    mainTitle.textContent = session.toolName + ' — ' + session.contentType;
    contentArea.innerHTML = '';

    const renderer = getRenderer(session.contentType);
    renderer(contentArea, session.data, session.meta, session.toolArgs || {}, session.reviewRequired, function (decision) {
      onDecision(sessionId, decision);
    });
  }

  function renderEmpty() {
    mainTitle.textContent = 'MCP Mux';
    contentArea.innerHTML = '<div class="empty-state">Waiting for preview data...</div>';
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
    stopHeartbeat();
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

    // Clean up local state
    sessions.delete(sessionId);
    if (sessionId === activeSessionId) {
      activeSessionId = null;

      // Advance to next queued session
      var nextId = null;
      while (queuedSessionIds.length > 0) {
        var candidate = queuedSessionIds.shift();
        if (sessions.has(candidate)) {
          nextId = candidate;
          break;
        }
      }
      updateQueueBadge();

      if (nextId) {
        selectSession(nextId);
      } else {
        var remaining = Array.from(sessions.keys());
        if (remaining.length > 0) {
          selectSession(remaining[remaining.length - 1]);
        } else {
          renderEmpty();
        }
      }
    }
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

  // --- Queue Badge ---

  var queueBadge = document.createElement('div');
  queueBadge.id = 'queue-badge';
  queueBadge.style.cssText = 'display:none;position:fixed;top:12px;right:12px;background:#6366f1;color:#fff;' +
    'font-size:13px;font-weight:600;padding:4px 10px;border-radius:12px;z-index:9999;' +
    'box-shadow:0 2px 8px rgba(99,102,241,0.3);pointer-events:none;font-family:inherit;';
  document.body.appendChild(queueBadge);

  function updateQueueBadge() {
    var validCount = 0;
    for (var i = 0; i < queuedSessionIds.length; i++) {
      if (sessions.has(queuedSessionIds[i])) validCount++;
    }
    if (validCount > 0) {
      queueBadge.textContent = validCount + ' waiting';
      queueBadge.style.display = 'block';
    } else {
      queueBadge.style.display = 'none';
    }
  }

  // --- Init ---

  renderEmpty();
  initTauri();
})();
