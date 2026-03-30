// @ts-nocheck
/* Citation slideout panel — shows detail for a citation */

(function () {
  'use strict';

  var panelEl = null;
  var overlayEl = null;

  function ensurePanel() {
    if (panelEl) return;

    overlayEl = document.createElement('div');
    overlayEl.className = 'citation-slideout-overlay';
    overlayEl.addEventListener('click', closeCitationPanel);
    document.body.appendChild(overlayEl);

    panelEl = document.createElement('div');
    panelEl.className = 'citation-slideout';
    document.body.appendChild(panelEl);
  }

  function openCitationPanel(type, data) {
    ensurePanel();
    panelEl.innerHTML = '';

    var utils = window.__companionUtils;
    var color = utils.CITATION_COLORS[type] || utils.CITATION_COLORS.doc;

    // ── Header ──
    var header = document.createElement('div');
    header.className = 'citation-slideout-header';

    var titleSpan = document.createElement('span');
    titleSpan.className = 'cite-panel-title';
    titleSpan.appendChild(utils.createBadge(color.label, color.bg, color.hex));

    var displayName = data.name || data.title || data.tableName || data.path || '';
    if (displayName) {
      var nameSpan = document.createElement('span');
      nameSpan.textContent = displayName;
      titleSpan.appendChild(nameSpan);
    }
    header.appendChild(titleSpan);

    var closeBtn = document.createElement('button');
    closeBtn.textContent = '\u2715';
    closeBtn.className = 'cite-panel-close';
    closeBtn.addEventListener('click', closeCitationPanel);
    header.appendChild(closeBtn);

    panelEl.appendChild(header);

    // ── Body ──
    var body = document.createElement('div');
    body.className = 'citation-slideout-body';

    var renderer = DETAIL_RENDERERS[type] || renderGenericDetail;
    renderer(body, data);

    panelEl.appendChild(body);

    // Show
    panelEl.classList.add('open');
    overlayEl.classList.add('open');
  }

  function closeCitationPanel() {
    if (panelEl) panelEl.classList.remove('open');
    if (overlayEl) overlayEl.classList.remove('open');
  }

  // ── Code detail ──
  function renderCodeDetail(body, data) {
    var utils = window.__companionUtils;

    // File path + line range
    var pathDiv = document.createElement('div');
    pathDiv.className = 'cite-panel-path cite-panel-path-blue';
    pathDiv.textContent = (data.file_path || '') + (data.line_start ? ' L' + data.line_start + '-' + (data.line_end || '') : '');
    body.appendChild(pathDiv);

    // Unit type + exported + complexity badges
    var badgeRow = document.createElement('div');
    badgeRow.className = 'cite-panel-badge-row';
    if (data.unit_type) badgeRow.appendChild(utils.createBadge(data.unit_type.toUpperCase(), 'var(--cite-code-bg)', 'var(--cite-code)'));
    if (data.exported) badgeRow.appendChild(utils.createBadge('EXPORTED', 'var(--color-success-bg)', 'var(--color-success-text)'));
    if (data.complexity) {
      var level = data.complexity <= 5 ? 'LOW' : data.complexity <= 15 ? 'MEDIUM' : 'HIGH';
      var cBg = data.complexity <= 5 ? 'var(--color-success-bg)' : data.complexity <= 15 ? 'var(--color-warning-bg)' : 'var(--color-error-bg)';
      var cText = data.complexity <= 5 ? 'var(--color-success-text)' : data.complexity <= 15 ? 'var(--color-warning-text)' : 'var(--color-error-text)';
      badgeRow.appendChild(utils.createBadge('COMPLEXITY: ' + data.complexity + ' (' + level + ')', cBg, cText));
    }
    body.appendChild(badgeRow);

    // Source code with line numbers
    var source = data.source || data.content || data.preview;
    if (source) {
      var pre = document.createElement('pre');
      pre.className = 'md-codeblock';

      var lines = source.split('\n');
      var startLine = data.line_start || 1;
      var html = lines.map(function (line, i) {
        var lineNum = '<span class="cite-panel-line-num">' + (startLine + i) + '</span>';
        return lineNum + utils.escapeHtml(line);
      }).join('\n');

      pre.innerHTML = html;
      body.appendChild(pre);
    }

    // Patterns
    if (data.patterns && data.patterns.length) {
      var patDiv = document.createElement('div');
      patDiv.className = 'cite-panel-badge-row';
      patDiv.style.marginTop = '12px';
      data.patterns.forEach(function (p) {
        patDiv.appendChild(utils.createBadge(typeof p === 'string' ? p : (p.name || String(p)), 'var(--bg-surface-inset)', 'var(--text-secondary)'));
      });
      body.appendChild(patDiv);
    }
  }

  // ── Document detail ──
  function renderDocDetail(body, data) {
    var utils = window.__companionUtils;

    if (data.status) {
      body.appendChild(utils.createStatusBadge(data.status));
      var spacer = document.createElement('div');
      spacer.className = 'cite-panel-spacer';
      body.appendChild(spacer);
    }

    if (data.content) {
      var md = utils.renderMarkdown(data.content);
      if (md instanceof HTMLElement) {
        body.appendChild(md);
      } else {
        var div = document.createElement('div');
        div.innerHTML = md;
        body.appendChild(div);
      }
    }
  }

  // ── Data governance detail ──
  function renderDgDetail(body, data) {
    var utils = window.__companionUtils;

    // Show data source name
    var dsName = data.dataSourceName || data.data_source_name || '';
    if (dsName) {
      var ds = document.createElement('div');
      ds.className = 'cite-panel-path cite-panel-path-green';
      ds.textContent = dsName + '.' + (data.name || data.tableName || data.table_name || '');
      body.appendChild(ds);
    }

    // If we already have columns, render them
    if (data.columns && data.columns.length) {
      renderDgColumnsTable(body, data.columns, data.metadataColumns, utils);
      return;
    }

    // No columns — fetch on-demand via companion proxy
    if (!utils.isProxyConfigured || !utils.isProxyConfigured()) return;

    var tableName = data.name || data.tableName || data.table_name || '';
    utils.proxyFetchWithStatus(body, 'get_data_schema', { table_name: tableName, include_metadata: true }, 'Loading schema...')
      .then(function (parsed) {
        if (!parsed || !parsed.data || !parsed.data.tables || !parsed.data.tables.length) return;

        // Find matching table by id or name
        var tableData = null;
        for (var i = 0; i < parsed.data.tables.length; i++) {
          var t = parsed.data.tables[i];
          if (t.id === data.id || t.name === tableName) { tableData = t; break; }
        }
        if (!tableData) tableData = parsed.data.tables[0];

        // Full path header if missing
        if (tableData.dataSource && tableData.dataSource.name && !dsName) {
          var dsEl = document.createElement('div');
          dsEl.className = 'cite-panel-path cite-panel-path-green';
          dsEl.textContent = tableData.dataSource.name + '.' + tableData.name;
          body.insertBefore(dsEl, body.firstChild);
        }

        // Source type badge
        if (tableData.dataSource && tableData.dataSource.sourceType) {
          var typeBadge = document.createElement('div');
          typeBadge.className = 'cite-panel-spacer-badge';
          typeBadge.appendChild(utils.createBadge(tableData.dataSource.sourceType, 'var(--bg-surface-inset)', 'var(--text-secondary)'));
          body.appendChild(typeBadge);
        }

        if (tableData.columns && tableData.columns.length) {
          renderDgColumnsTable(body, tableData.columns, tableData.metadataColumns, utils);
        }
      });
  }

  // ── Render DG columns as a table with metadata column headers ──
  function renderDgColumnsTable(body, columns, metadataColumns, utils) {
    var table = document.createElement('table');
    table.className = 'cite-panel-table';

    // Build header — standard columns + metadata columns
    var thead = document.createElement('thead');
    var headerRow = document.createElement('tr');
    var headers = ['Column', 'Type', 'PK', 'Description'];
    headers.forEach(function (h) {
      var th = document.createElement('th');
      th.className = 'cite-panel-th';
      th.textContent = h;
      headerRow.appendChild(th);
    });

    // Add metadata column headers if present
    if (metadataColumns && metadataColumns.length) {
      metadataColumns.forEach(function (mc) {
        var th = document.createElement('th');
        th.className = 'cite-panel-th cite-panel-th-green';
        th.textContent = mc.name || '';
        if (mc.isSystemColumn) {
          var sysIcon = document.createElement('span');
          sysIcon.style.cssText = 'font-size:9px;margin-left:3px;opacity:0.6;';
          sysIcon.textContent = '\u2022';
          th.appendChild(sysIcon);
        }
        headerRow.appendChild(th);
      });
    }
    thead.appendChild(headerRow);
    table.appendChild(thead);

    // Build rows
    var tbody = document.createElement('tbody');
    columns.forEach(function (col) {
      var tr = document.createElement('tr');
      tr.className = 'cite-panel-row';

      // Column name
      var tdName = document.createElement('td');
      tdName.className = 'cite-panel-td cite-panel-td-name';
      tdName.textContent = col.name || '';
      tr.appendChild(tdName);

      // Data type
      var tdType = document.createElement('td');
      tdType.className = 'cite-panel-td cite-panel-td-type';
      tdType.textContent = col.originalDataType || col.dataType || col.data_type || '';
      tr.appendChild(tdType);

      // Primary key
      var tdPk = document.createElement('td');
      tdPk.className = 'cite-panel-td cite-panel-td-center';
      if (col.isPrimaryKey || col.is_primary_key) {
        tdPk.appendChild(utils.createBadge('PK', 'var(--color-warning-bg)', 'var(--color-warning-text)'));
      }
      tr.appendChild(tdPk);

      // Description
      var tdDesc = document.createElement('td');
      tdDesc.className = 'cite-panel-td cite-panel-td-desc';
      tdDesc.textContent = col.description || '';
      tr.appendChild(tdDesc);

      // Metadata value cells (empty for now — values require include_values=true)
      if (metadataColumns && metadataColumns.length) {
        metadataColumns.forEach(function () {
          var tdMeta = document.createElement('td');
          tdMeta.className = 'cite-panel-td cite-panel-td-meta';
          tdMeta.textContent = '\u2014';
          tr.appendChild(tdMeta);
        });
      }

      tbody.appendChild(tr);
    });
    table.appendChild(tbody);
    body.appendChild(table);

    // Column count footer
    var footer = document.createElement('div');
    footer.className = 'cite-panel-footer';
    footer.textContent = columns.length + ' column' + (columns.length !== 1 ? 's' : '');
    if (metadataColumns && metadataColumns.length) {
      footer.textContent += ' \u00b7 ' + metadataColumns.length + ' metadata field' + (metadataColumns.length !== 1 ? 's' : '');
    }
    body.appendChild(footer);
  }

  // ── API endpoint detail ──
  function renderApiDetail(body, data) {
    var utils = window.__companionUtils;

    var methodRow = document.createElement('div');
    methodRow.className = 'cite-panel-method-row';

    var method = (data.method || 'GET').toUpperCase();
    var mColor = utils.HTTP_METHOD_COLORS[method] || { hex: 'var(--text-secondary)', bg: 'var(--bg-surface-inset)', text: 'var(--text-secondary)' };
    methodRow.appendChild(utils.createBadge(method, mColor.bg, mColor.text));

    var pathSpan = document.createElement('span');
    pathSpan.className = 'cite-panel-mono-path';
    pathSpan.textContent = data.path || '';
    methodRow.appendChild(pathSpan);
    body.appendChild(methodRow);

    if (data.description) {
      var desc = document.createElement('p');
      desc.className = 'cite-panel-desc cite-panel-desc-margin';
      desc.textContent = data.description;
      body.appendChild(desc);
    }

    if (data.repositoryName) {
      var repo = document.createElement('div');
      repo.className = 'cite-panel-repo';
      repo.textContent = 'Repository: ' + data.repositoryName;
      body.appendChild(repo);
    }
  }

  // ── Knowledge Dex detail ──
  function renderKdexDetail(body, data) {
    var utils = window.__companionUtils;

    // Scope badge
    var scope = data.scope || '';
    if (scope) {
      var scopeBg = scope === 'ORGANIZATIONAL' ? 'var(--color-success-bg)' : 'var(--cite-code-bg)';
      var scopeText = scope === 'ORGANIZATIONAL' ? 'var(--color-success)' : 'var(--accent-primary)';
      var scopeLabel = scope === 'ORGANIZATIONAL' ? 'Organization' : 'Personal';
      body.appendChild(utils.createBadge(scopeLabel, scopeBg, scopeText));
      var spacer = document.createElement('div');
      spacer.className = 'cite-panel-spacer-sm';
      body.appendChild(spacer);
    }

    // Parent concept path
    if (data.parentName) {
      var parentPath = document.createElement('div');
      parentPath.className = 'cite-panel-parent-path';
      parentPath.textContent = data.parentName + ' \u2192 ' + (data.name || '');
      body.appendChild(parentPath);
    }

    // Description
    if (data.description) {
      var desc = document.createElement('p');
      desc.className = 'cite-panel-desc cite-panel-desc-margin-bottom';
      desc.textContent = data.description;
      body.appendChild(desc);
    }

    // If we already have attributes, render them
    if (data.attributes && data.attributes.length) {
      renderKdexAttributesTable(body, data.attributes, data.mappings, utils);
      return;
    }

    // Fetch concept with attributes on-demand via companion proxy
    if (!utils.isProxyConfigured || !utils.isProxyConfigured()) return;

    var conceptName = data.name || '';
    utils.proxyFetchWithStatus(body, 'get_business_concepts', { name_pattern: conceptName, include_mappings: true }, 'Loading attributes...')
      .then(function (parsed) {
        if (!parsed || !parsed.data || !parsed.data.concepts) return;

        // Find matching concept by id or exact name
        var concept = null;
        for (var i = 0; i < parsed.data.concepts.length; i++) {
          var c = parsed.data.concepts[i];
          if (c.id === data.id) { concept = c; break; }
          if (c.name === conceptName) { concept = c; }
        }

        if (!concept) {
          var notFound = document.createElement('div');
          notFound.className = 'cite-panel-not-found';
          notFound.textContent = 'Concept details not found';
          body.appendChild(notFound);
          return;
        }

        // Description if not already shown
        if (concept.description && !data.description) {
          var descEl = document.createElement('p');
          descEl.className = 'cite-panel-desc cite-panel-desc-margin-bottom';
          descEl.textContent = concept.description;
          body.appendChild(descEl);
        }

        // Attributes table
        if (concept.attributes && concept.attributes.length) {
          renderKdexAttributesTable(body, concept.attributes, concept.mappings, utils);
        } else {
          var noAttrs = document.createElement('div');
          noAttrs.className = 'cite-panel-not-found';
          noAttrs.style.padding = '8px 0';
          noAttrs.textContent = 'No attributes defined';
          body.appendChild(noAttrs);
        }

        // Column mappings
        if (concept.mappings && concept.mappings.length) {
          renderKdexMappings(body, concept.mappings, utils);
        }
      });
  }

  // ── Render KDex attributes as a table ──
  function renderKdexAttributesTable(body, attributes, mappings, utils) {
    var heading = document.createElement('div');
    heading.className = 'cite-panel-section-heading cite-panel-section-heading-teal';
    heading.textContent = 'Attributes (' + attributes.length + ')';
    body.appendChild(heading);

    var table = document.createElement('table');
    table.className = 'cite-panel-table';

    var thead = document.createElement('thead');
    var headerRow = document.createElement('tr');
    ['Attribute', 'Description'].forEach(function (h) {
      var th = document.createElement('th');
      th.className = 'cite-panel-th';
      th.textContent = h;
      headerRow.appendChild(th);
    });
    thead.appendChild(headerRow);
    table.appendChild(thead);

    var tbody = document.createElement('tbody');
    attributes.forEach(function (attr) {
      var tr = document.createElement('tr');
      tr.className = 'cite-panel-row';

      var tdName = document.createElement('td');
      tdName.className = 'cite-panel-td cite-panel-td-name';
      tdName.textContent = attr.name || '';
      tr.appendChild(tdName);

      var tdDesc = document.createElement('td');
      tdDesc.className = 'cite-panel-td cite-panel-td-desc';
      tdDesc.textContent = attr.description || '';
      tr.appendChild(tdDesc);

      tbody.appendChild(tr);
    });
    table.appendChild(tbody);
    body.appendChild(table);
  }

  // ── Render KDex column mappings ──
  function renderKdexMappings(body, mappings, utils) {
    var heading = document.createElement('div');
    heading.className = 'cite-panel-section-heading cite-panel-section-heading-gray';
    heading.textContent = 'Column Mappings';
    body.appendChild(heading);

    mappings.forEach(function (m) {
      var row = document.createElement('div');
      row.className = 'cite-panel-mapping-row';

      var col = document.createElement('span');
      col.className = 'cite-panel-mapping-col';
      col.textContent = (m.dataSourceName || '') + '.' + (m.tableName || '') + '.' + (m.columnName || m.column_name || '');
      row.appendChild(col);

      body.appendChild(row);
    });
  }

  // ── Generic / fallback detail ──
  function renderGenericDetail(body, data) {
    var pre = document.createElement('pre');
    pre.className = 'md-codeblock';
    pre.textContent = JSON.stringify(data, null, 2);
    body.appendChild(pre);
  }

  // ── Detail renderer map (extensible without if/else) ──
  var DETAIL_RENDERERS = {
    code: renderCodeDetail,
    doc: renderDocDetail,
    dg: renderDgDetail,
    api: renderApiDetail,
    kdex: renderKdexDetail
  };

  // Register on shared utils
  window.__companionUtils = window.__companionUtils || {};
  window.__companionUtils.openCitationPanel = openCitationPanel;
  window.__companionUtils.closeCitationPanel = closeCitationPanel;
})();
