// @ts-nocheck
/* MCPViews — Plugin Manager UI
 * Standalone window for browsing, installing, and managing MCP plugins.
 */

(function () {
  'use strict';

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
      var entries = await window.__TAURI__.core.invoke('fetch_registry', { registryUrl: null });
      var installed = await window.__TAURI__.core.invoke('list_plugins');
      var installedMap = {};
      installed.forEach(function (p) { installedMap[p.name] = p; });
      renderRegistryCards(container, entries, installedMap);
    } catch (e) {
      container.innerHTML = renderEmptyState(
        'Unable to load registry',
        escapeHtml(String(e)) + '<br><br>Check your internet connection, or add a registry source in the <strong>Settings</strong> tab.'
      );
    }
  }

  function renderRegistryCards(container, entries, installedMap) {
    if (!entries || entries.length === 0) {
      container.innerHTML = renderEmptyState(
        'No plugins available',
        'Add a registry source in the <strong>Settings</strong> tab, or use <strong>Add Custom Plugin</strong> in the Installed tab to add plugins manually.'
      );
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
      var installedPlugin = installedMap[name];
      var isInstalled = !!installedPlugin;
      var hasUpdate = installedPlugin && installedPlugin.update_available;

      var buttonHtml = hasUpdate
        ? '<button class="btn btn-primary update-btn">Update to v' + escapeHtml(installedPlugin.update_available) + '</button>'
        : isInstalled
          ? '<button class="btn btn-muted" disabled>Installed</button>'
          : '<button class="btn btn-primary install-btn">Install</button>';

      card.innerHTML =
        '<div class="plugin-name">' + escapeHtml(name) + '</div>' +
        '<div class="plugin-description">' + escapeHtml(description) + '</div>' +
        (version ? '<div class="plugin-version">v' + escapeHtml(version) + '</div>' : '') +
        '<div style="margin-top:8px;">' + buttonHtml + '</div>';

      if (!isInstalled) {
        card.querySelector('.install-btn').addEventListener('click', function () {
          installPlugin(entry);
        });
      }

      var updateBtn = card.querySelector('.update-btn');
      if (updateBtn) {
        updateBtn.addEventListener('click', function () {
          updatePlugin(name);
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
      container.innerHTML = renderEmptyState(
        'No plugins installed',
        'Browse the <strong>Registry</strong> tab to find plugins, or use the button below to add a custom plugin.'
      );
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

        var updateBadgeHtml = '';
        if (plugin.update_available) {
          updateBadgeHtml = '<span class="auth-badge" style="background:#1e1b4b;color:#818cf8">v' + escapeHtml(plugin.update_available) + ' available</span>';
        }

        var actionsHtml = '';
        if (plugin.update_available) {
          actionsHtml += '<button class="btn btn-primary update-btn">Update</button>';
        }
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
              (updateBadgeHtml ? ' ' + updateBadgeHtml : '') +
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

        var updateBtn = row.querySelector('.update-btn');
        if (updateBtn) {
          updateBtn.addEventListener('click', function () {
            updatePlugin(plugin.name);
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
      var btn = document.querySelector('.install-btn:not([disabled])');
      if (btn) {
        btn.disabled = true;
        btn.textContent = entry.download_url ? 'Downloading...' : 'Installing...';
      }

      if (entry.download_url) {
        await window.__TAURI__.core.invoke('install_plugin_from_registry', {
          entryJson: JSON.stringify(entry),
        });
      } else {
        await window.__TAURI__.core.invoke('install_plugin', {
          manifestJson: JSON.stringify(entry.manifest),
        });
      }
      loadRegistry();
      loadInstalled();

      // Auto-prompt auth if the plugin requires it
      var auth = entry.manifest && entry.manifest.mcp && entry.manifest.mcp.auth;
      if (auth) {
        promptAuth(entry.manifest.name, auth);
      }
    } catch (e) {
      showNotification('Failed to install plugin: ' + e, true);
      loadRegistry(); // Reset button states
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

  async function updatePlugin(name) {
    try {
      await window.__TAURI__.core.invoke('update_plugin', { name: name });
      showNotification('Plugin ' + name + ' updated');
      loadInstalled();
      loadRegistry();
    } catch (e) {
      alert('Failed to update plugin: ' + e);
    }
  }

  async function addCustomPlugin() {
    try {
      var path = await window.__TAURI__.dialog.open({
        filters: [
          { name: 'Plugin Package', extensions: ['zip', 'json'] },
        ],
        multiple: false,
      });
      if (path) {
        if (path.endsWith('.zip')) {
          await window.__TAURI__.core.invoke('install_plugin_from_zip', { path: path });
        } else {
          await window.__TAURI__.core.invoke('install_plugin_from_file', { path: path });
        }
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
    var container = document.getElementById('tab-settings');
    container.innerHTML = '<div class="loading">Loading settings...</div>';

    try {
      var sources = await window.__TAURI__.core.invoke('get_registry_sources');
      renderSourcesList(container, sources);
    } catch (e) {
      container.innerHTML = '<div class="empty-state">Failed to load settings: ' + escapeHtml(String(e)) + '</div>';
    }
  }

  function renderSourcesList(container, sources) {
    container.innerHTML = '';

    var section = document.createElement('div');
    section.className = 'settings-section';
    section.style.maxWidth = '600px';

    var title = document.createElement('label');
    title.textContent = 'Registry Sources';
    title.style.marginBottom = '12px';
    title.style.display = 'block';
    section.appendChild(title);

    var list = document.createElement('div');
    list.className = 'installed-list';

    sources.forEach(function(source) {
      var row = document.createElement('div');
      row.className = 'installed-row';
      row.innerHTML =
        '<div class="plugin-info" style="flex:1">' +
          '<div class="plugin-name">' + escapeHtml(source.name) + '</div>' +
          '<div class="plugin-meta" style="word-break:break-all">' + escapeHtml(source.url) + '</div>' +
        '</div>' +
        '<div class="plugin-actions">' +
          '<button class="btn ' + (source.enabled ? 'btn-primary' : 'btn-secondary') + ' toggle-btn">' +
            (source.enabled ? 'Enabled' : 'Disabled') +
          '</button>' +
          '<button class="btn btn-danger remove-source-btn">Remove</button>' +
        '</div>';

      row.querySelector('.toggle-btn').addEventListener('click', function() {
        toggleSource(source.url);
      });
      row.querySelector('.remove-source-btn').addEventListener('click', function() {
        removeSource(source.url);
      });

      list.appendChild(row);
    });

    section.appendChild(list);

    // Add source form
    var addForm = document.createElement('div');
    addForm.style.marginTop = '16px';
    addForm.innerHTML =
      '<div style="display:flex;gap:8px;align-items:flex-end">' +
        '<div style="flex:1">' +
          '<label style="font-size:12px;color:#737373;display:block;margin-bottom:4px">Name</label>' +
          '<input type="text" id="new-source-name" placeholder="My Registry" style="width:100%;padding:8px 12px;background:#1a1a1a;border:1px solid #2a2a2a;border-radius:6px;color:#e5e5e5;font-size:13px;font-family:inherit;outline:none" />' +
        '</div>' +
        '<div style="flex:2">' +
          '<label style="font-size:12px;color:#737373;display:block;margin-bottom:4px">URL</label>' +
          '<input type="text" id="new-source-url" placeholder="https://example.com/registry.json" style="width:100%;padding:8px 12px;background:#1a1a1a;border:1px solid #2a2a2a;border-radius:6px;color:#e5e5e5;font-size:13px;font-family:inherit;outline:none" />' +
        '</div>' +
        '<button class="btn btn-primary" id="add-source-btn">Add</button>' +
      '</div>';
    section.appendChild(addForm);

    container.appendChild(section);

    document.getElementById('add-source-btn').addEventListener('click', function() {
      addSource();
    });
  }

  async function addSource() {
    var name = document.getElementById('new-source-name').value.trim();
    var url = document.getElementById('new-source-url').value.trim();
    if (!name || !url) {
      showNotification('Name and URL are required', true);
      return;
    }
    try {
      await window.__TAURI__.core.invoke('add_registry_source', { name: name, url: url });
      showNotification('Registry source added');
      loadSettings();
    } catch (e) {
      showNotification('Failed to add source: ' + e, true);
    }
  }

  async function removeSource(url) {
    try {
      await window.__TAURI__.core.invoke('remove_registry_source', { url: url });
      showNotification('Registry source removed');
      loadSettings();
    } catch (e) {
      showNotification('Failed to remove source: ' + e, true);
    }
  }

  async function toggleSource(url) {
    try {
      await window.__TAURI__.core.invoke('toggle_registry_source', { url: url });
      loadSettings();
    } catch (e) {
      showNotification('Failed to toggle source: ' + e, true);
    }
  }

  // --- Utility ---

  function renderEmptyState(title, message) {
    return '<div class="empty-state" style="flex-direction:column;gap:12px;text-align:center">' +
      '<div style="font-size:18px;margin-bottom:4px">' + title + '</div>' +
      '<div style="font-size:12px;color:#737373;max-width:360px">' + message + '</div>' +
      '</div>';
  }

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
