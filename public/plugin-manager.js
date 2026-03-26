// @ts-nocheck
/* MCP Mux — Plugin Manager UI
 * Standalone window for browsing, installing, and managing MCP plugins.
 */

(function () {
  'use strict';

  var DEFAULT_REGISTRY_URL = 'https://raw.githubusercontent.com/anthropics/mcp-registry/main/registry.json';

  // --- Tab Switching ---

  window.switchTab = function switchTab(tabName) {
    // Update tab bar
    var tabs = document.querySelectorAll('.tab');
    for (var i = 0; i < tabs.length; i++) {
      if (tabs[i].getAttribute('data-tab') === tabName) {
        tabs[i].classList.add('active');
      } else {
        tabs[i].classList.remove('active');
      }
    }

    // Update tab content
    var contents = document.querySelectorAll('.tab-content');
    for (var j = 0; j < contents.length; j++) {
      if (contents[j].id === 'tab-' + tabName) {
        contents[j].classList.add('active');
      } else {
        contents[j].classList.remove('active');
      }
    }

    // Load data for the active tab
    if (tabName === 'registry') {
      loadRegistry();
    } else if (tabName === 'installed') {
      loadInstalled();
    } else if (tabName === 'settings') {
      loadSettings();
    }
  };

  // --- Registry Tab ---

  async function loadRegistry() {
    var container = document.getElementById('tab-registry');
    container.innerHTML = '<div class="loading">Loading registry...</div>';

    try {
      var settings = await window.__TAURI__.core.invoke('get_settings');
      var registryUrl = (settings && settings.registry_url) || null;
      var entries = await window.__TAURI__.core.invoke('fetch_registry', { registryUrl: registryUrl });
      var installed = await window.__TAURI__.core.invoke('list_plugins');
      var installedNames = new Set(installed.map(function (p) { return p.name; }));
      renderRegistryCards(container, entries, installedNames);
    } catch (e) {
      container.innerHTML = '<div class="empty-state">Failed to load registry: ' + escapeHtml(String(e)) + '</div>';
    }
  }

  function renderRegistryCards(container, entries, installedNames) {
    if (!entries || entries.length === 0) {
      container.innerHTML = '<div class="empty-state">No plugins found in registry.</div>';
      return;
    }

    var grid = document.createElement('div');
    grid.className = 'registry-grid';

    entries.forEach(function (entry) {
      var card = document.createElement('div');
      card.className = 'plugin-card';

      var name = entry.manifest ? entry.manifest.name : (entry.name || 'Unknown');
      var description = entry.manifest ? (entry.manifest.description || '') : (entry.description || '');
      var version = entry.manifest ? (entry.manifest.version || '') : (entry.version || '');
      var isInstalled = installedNames.has(name);

      card.innerHTML =
        '<div class="plugin-name">' + escapeHtml(name) + '</div>' +
        '<div class="plugin-description">' + escapeHtml(description) + '</div>' +
        (version ? '<div class="plugin-version">v' + escapeHtml(version) + '</div>' : '') +
        '<div style="margin-top:8px;">' +
          (isInstalled
            ? '<button class="btn btn-muted" disabled>Installed</button>'
            : '<button class="btn btn-primary install-btn">Install</button>') +
        '</div>';

      if (!isInstalled) {
        card.querySelector('.install-btn').addEventListener('click', function () {
          installPlugin(entry);
        });
      }

      grid.appendChild(card);
    });

    container.innerHTML = '';
    container.appendChild(grid);
  }

  // --- Installed Tab ---

  async function loadInstalled() {
    var container = document.getElementById('tab-installed');
    container.innerHTML = '<div class="loading">Loading installed plugins...</div>';

    try {
      var plugins = await window.__TAURI__.core.invoke('list_plugins');
      renderInstalledList(container, plugins);
    } catch (e) {
      container.innerHTML = '<div class="empty-state">Failed to load plugins: ' + escapeHtml(String(e)) + '</div>';
    }
  }

  function renderInstalledList(container, plugins) {
    container.innerHTML = '';

    if (!plugins || plugins.length === 0) {
      container.innerHTML = '<div class="empty-state">No plugins installed.</div>';
    } else {
      var list = document.createElement('div');
      list.className = 'installed-list';

      plugins.forEach(function (plugin) {
        var row = document.createElement('div');
        row.className = 'installed-row';

        var hasAuth = !!plugin.auth_type;
        var authConfigured = plugin.auth_configured !== false;

        var authBadgeHtml = '';
        if (hasAuth) {
          authBadgeHtml = authConfigured
            ? '<span class="auth-badge configured">Auth OK</span>'
            : '<span class="auth-badge not-configured">Auth Needed</span>';
        }

        var actionsHtml = '';
        if (hasAuth && !authConfigured) {
          actionsHtml += '<button class="btn btn-secondary configure-auth-btn">Configure Auth</button>';
        }
        actionsHtml += '<button class="btn btn-danger remove-btn">Remove</button>';

        row.innerHTML =
          '<div class="plugin-info">' +
            '<div class="plugin-name">' + escapeHtml(plugin.name || 'Unknown') + '</div>' +
            '<div class="plugin-meta">' +
              (plugin.version ? 'v' + escapeHtml(plugin.version) : '') +
              (authBadgeHtml ? ' ' + authBadgeHtml : '') +
            '</div>' +
          '</div>' +
          '<div class="plugin-actions">' + actionsHtml + '</div>';

        var removeBtn = row.querySelector('.remove-btn');
        if (removeBtn) {
          removeBtn.addEventListener('click', function () {
            removePlugin(plugin.name);
          });
        }

        var authBtn = row.querySelector('.configure-auth-btn');
        if (authBtn) {
          authBtn.addEventListener('click', function () {
            configureAuth(plugin.name);
          });
        }

        list.appendChild(row);
      });

      container.appendChild(list);
    }

    // Add custom plugin button
    var addRow = document.createElement('div');
    addRow.className = 'add-custom-row';
    addRow.innerHTML = '<button class="btn btn-secondary" id="add-custom-btn">Add Custom Plugin...</button>';
    container.appendChild(addRow);

    document.getElementById('add-custom-btn').addEventListener('click', function () {
      addCustomPlugin();
    });
  }

  // --- Actions ---

  async function installPlugin(entry) {
    try {
      await window.__TAURI__.core.invoke('install_plugin', {
        manifestJson: JSON.stringify(entry.manifest),
      });
      loadRegistry();
      loadInstalled();

      // Auto-prompt auth if the plugin requires it
      var auth = entry.manifest.mcp && entry.manifest.mcp.auth;
      if (auth) {
        promptAuth(entry.manifest.name, auth);
      }
    } catch (e) {
      alert('Failed to install plugin: ' + e);
    }
  }

  async function removePlugin(name) {
    try {
      await window.__TAURI__.core.invoke('uninstall_plugin', { name: name });
      loadRegistry();
      loadInstalled();
    } catch (e) {
      alert('Failed to remove plugin: ' + e);
    }
  }

  async function addCustomPlugin() {
    try {
      var path = await window.__TAURI__.dialog.open({
        filters: [{ name: 'JSON Manifest', extensions: ['json'] }],
        multiple: false,
      });
      if (path) {
        await window.__TAURI__.core.invoke('install_plugin_from_file', { path: path });
        loadInstalled();
      }
    } catch (e) {
      alert('Failed to add custom plugin: ' + e);
    }
  }

  async function configureAuth(pluginName) {
    try {
      var plugins = await window.__TAURI__.core.invoke('list_plugins');
      var plugin = plugins.find(function (p) { return p.name === pluginName; });
      if (!plugin || !plugin.auth_type) {
        showNotification('No auth configuration found for ' + pluginName, true);
        return;
      }
      promptAuth(pluginName, { type: plugin.auth_type });
    } catch (e) {
      showNotification('Auth error: ' + e, true);
    }
  }

  async function promptAuth(pluginName, auth) {
    if (auth.type === 'oauth') {
      try {
        await window.__TAURI__.core.invoke('start_plugin_auth', { pluginName: pluginName });
        showNotification('Authentication configured for ' + pluginName);
        loadInstalled();
      } catch (e) {
        showNotification('Auth error: ' + e, true);
      }
    } else {
      showTokenModal(pluginName, auth);
    }
  }

  function showNotification(msg, isError) {
    var el = document.createElement('div');
    el.className = 'notification' + (isError ? ' notification-error' : '');
    el.textContent = msg;
    document.body.appendChild(el);
    setTimeout(function () {
      el.style.opacity = '0';
      setTimeout(function () { document.body.removeChild(el); }, 300);
    }, 3000);
  }

  function showTokenModal(pluginName, auth) {
    var label = auth.type === 'bearer' ? 'Bearer Token' : 'API Key';
    var overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML =
      '<div class="modal">' +
        '<div class="modal-title">Configure ' + escapeHtml(label) + '</div>' +
        '<div class="modal-body">' +
          '<p>Enter your ' + escapeHtml(label.toLowerCase()) + ' for <strong>' + escapeHtml(pluginName) + '</strong>:</p>' +
          '<input type="password" class="modal-input" id="token-input" placeholder="Paste ' + escapeHtml(label.toLowerCase()) + ' here..." />' +
        '</div>' +
        '<div class="modal-actions">' +
          '<button class="btn btn-secondary" id="modal-skip">Skip</button>' +
          '<button class="btn btn-primary" id="modal-save">Save</button>' +
        '</div>' +
      '</div>';
    document.body.appendChild(overlay);

    document.getElementById('modal-skip').addEventListener('click', function () {
      document.body.removeChild(overlay);
    });
    document.getElementById('modal-save').addEventListener('click', async function () {
      var token = document.getElementById('token-input').value.trim();
      if (!token) return;
      try {
        await window.__TAURI__.core.invoke('store_plugin_token', { pluginName: pluginName, token: token });
        document.body.removeChild(overlay);
        showNotification('Authentication configured for ' + pluginName);
        loadInstalled();
      } catch (e) {
        alert('Failed to save token: ' + e);
      }
    });
  }

  // --- Settings Tab ---

  async function loadSettings() {
    var input = document.getElementById('registry-url');
    try {
      var settings = await window.__TAURI__.core.invoke('get_settings');
      input.value = (settings && settings.registry_url) || DEFAULT_REGISTRY_URL;
    } catch (e) {
      input.value = DEFAULT_REGISTRY_URL;
    }
  }

  window.resetRegistryUrl = function resetRegistryUrl() {
    var input = document.getElementById('registry-url');
    input.value = DEFAULT_REGISTRY_URL;
    showSettingsMessage('Reset to default.', false);
  };

  window.saveSettings = async function saveSettings() {
    var input = document.getElementById('registry-url');
    var url = input.value.trim();
    if (!url) {
      showSettingsMessage('URL cannot be empty.', true);
      return;
    }
    try {
      await window.__TAURI__.core.invoke('save_settings', { settings: { registry_url: url } });
      showSettingsMessage('Settings saved.', false);
    } catch (e) {
      showSettingsMessage('Failed to save: ' + e, true);
    }
  };

  function showSettingsMessage(msg, isError) {
    var el = document.getElementById('settings-message');
    el.textContent = msg;
    el.className = isError ? 'error-msg' : '';
    el.style.color = isError ? '#ef4444' : '#22c55e';
    el.style.fontSize = '12px';
    el.style.marginTop = '8px';
    setTimeout(function () { el.textContent = ''; }, 3000);
  }

  // --- Utility ---

  function escapeHtml(str) {
    var div = document.createElement('div');
    div.appendChild(document.createTextNode(str));
    return div.innerHTML;
  }

  // --- Init ---

  function init() {
    if (!window.__TAURI__) {
      console.log('Tauri API not available');
      return;
    }
    loadRegistry();
  }

  init();
})();
