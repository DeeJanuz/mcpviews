// @ts-nocheck
/* Structured data table renderer — tabular data with hierarchical rows,
 * change tracking, sort/filter, and review mode with per-row/column
 * accept/reject and cell editing.
 *
 * Data shape:
 * {
 *   title: "Optional",
 *   tables: [{
 *     id: "t1",
 *     name: "Table Name",
 *     columns: [{ id: "c1", name: "Col", change: null|"add"|"delete" }],
 *     rows: [{
 *       id: "r1",
 *       cells: { "c1": { value: "v", change: null|"add"|"delete"|"update" } },
 *       children: []
 *     }]
 *   }]
 * }
 */

(function () {
  'use strict';

  window.__renderers = window.__renderers || {};

  var utils = window.__companionUtils || {};
  var escapeHtml = utils.escapeHtml || function (s) {
    var d = document.createElement('div');
    d.textContent = s;
    return d.innerHTML;
  };

  var sdu = window.__structuredDataUtils || {};
  var getCellValue = sdu.getCellValue;
  var getCellChange = sdu.getCellChange;
  var flattenRows = sdu.flattenRows;
  var sortRows = sdu.sortRows;
  var filterRows = sdu.filterRows;
  var createTableState = sdu.createTableState;
  var setAllRowDecisions = sdu.setAllRowDecisions;
  var buildDecisionPayload = sdu.buildDecisionPayload;
  var applyBulkDecision = sdu.applyBulkDecision;

  // ── 1. CSS Injection ──

  function injectStyles() {
    if (document.getElementById('structured-data-styles')) return;
    var style = document.createElement('style');
    style.id = 'structured-data-styles';
    style.textContent = [
      '.sd-container { background: var(--glass-bg-heavy); backdrop-filter: blur(var(--glass-blur)); border: 1px solid var(--glass-border); border-radius: var(--border-radius-lg); padding: var(--space-4); margin-bottom: var(--space-4); box-shadow: var(--glass-shadow); overflow-x: auto; }',
      '.sd-title { font-family: var(--font-sans); font-size: 18px; font-weight: var(--weight-semibold); color: var(--text-primary); margin: 0 0 var(--space-3) 0; }',
      '.sd-filter { font-family: var(--font-sans); font-size: var(--text-body); color: var(--text-primary); background: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--border-radius-sm); padding: var(--space-1) var(--space-2); margin-bottom: var(--space-3); width: 100%; box-sizing: border-box; }',
      '.sd-filter:focus { outline: none; border-color: var(--color-info); }',
      '.sd-table { width: 100%; border-collapse: collapse; font-family: var(--font-sans); font-size: var(--text-body); }',
      '.sd-th { text-align: left; padding: var(--space-2) var(--space-3); border-bottom: 2px solid var(--border-default); color: var(--text-secondary); font-weight: var(--weight-semibold); font-size: var(--text-small); cursor: pointer; user-select: none; white-space: nowrap; }',
      '.sd-th:hover { color: var(--text-primary); }',
      '.sd-td { padding: var(--space-2) var(--space-3); border-bottom: 1px solid var(--border-subtle); color: var(--text-primary); vertical-align: top; }',
      '.sd-cell-add { background: var(--color-success-bg); color: var(--color-success-text); }',
      '.sd-cell-delete { background: var(--color-error-bg); color: var(--color-error-text); text-decoration: line-through; }',
      '.sd-cell-update { background: var(--color-warning-bg); color: var(--color-warning-text); }',
      '.sd-cell-edited { background: var(--color-info-bg); color: var(--color-info-text); }',
      '.sd-col-add { border-bottom-color: var(--color-success) !important; color: var(--color-success-text); }',
      '.sd-col-delete { border-bottom-color: var(--color-error) !important; color: var(--color-error-text); text-decoration: line-through; }',
      '.sd-col-rejected { opacity: 0.4; }',
      '.sd-expand-toggle { cursor: pointer; border: none; background: transparent; color: var(--text-secondary); font-size: var(--text-small); padding: 0 var(--space-1); line-height: 1; }',
      '.sd-expand-toggle:hover { color: var(--text-primary); }',
      '.sd-depth-0 { padding-left: var(--space-3); }',
      '.sd-depth-1 { padding-left: calc(var(--space-3) + 16px); }',
      '.sd-depth-2 { padding-left: calc(var(--space-3) + 32px); }',
      '.sd-depth-3 { padding-left: calc(var(--space-3) + 48px); }',
      '.sd-depth-4 { padding-left: calc(var(--space-3) + 64px); }',
      '.sd-depth-5 { padding-left: calc(var(--space-3) + 80px); }',
      '.sd-sort-indicator { margin-left: var(--space-1); font-size: var(--text-xs); color: var(--text-tertiary); }',
      '.sd-decision-toggle { display: inline-flex; gap: 2px; margin-left: var(--space-2); }',
      '.sd-decision-toggle button { font-size: var(--text-xs); padding: 2px 6px; border: 1px solid var(--border-default); border-radius: var(--border-radius-sm); cursor: pointer; background: var(--bg-surface); color: var(--text-secondary); }',
      '.sd-decision-accept { background: var(--color-success-bg) !important; color: var(--color-success-text) !important; border-color: var(--color-success) !important; }',
      '.sd-decision-reject { background: var(--color-error-bg) !important; color: var(--color-error-text) !important; border-color: var(--color-error) !important; }',
      '.sd-submit-bar { position: sticky; bottom: 0; background: var(--glass-bg-heavy); backdrop-filter: blur(var(--glass-blur)); border-top: 1px solid var(--glass-border); padding: var(--space-3) var(--space-4); display: flex; gap: var(--space-2); justify-content: flex-end; align-items: center; margin-top: var(--space-4); border-radius: 0 0 var(--border-radius-lg) var(--border-radius-lg); }',
      '.sd-cell-editor { font-family: var(--font-sans); font-size: var(--text-body); color: var(--text-primary); background: var(--bg-surface); border: 1px solid var(--color-info); border-radius: var(--border-radius-sm); padding: var(--space-1) var(--space-2); width: 100%; box-sizing: border-box; outline: none; }',
      '.sd-empty { font-family: var(--font-sans); font-size: var(--text-body); color: var(--text-tertiary); padding: var(--space-6); text-align: center; }',
      '.sd-row-rejected { opacity: 0.4; }',
      '.sd-row-rejected .sd-td { background: var(--bg-surface-subtle); color: var(--text-tertiary); }',
      '.sd-table-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); margin-bottom: var(--space-2); }',
      '.sd-table-name { font-family: var(--font-sans); font-size: var(--text-body); font-weight: var(--weight-medium); color: var(--text-secondary); margin: 0; }',
      '.sd-csv-btn { flex-shrink: 0; padding: var(--space-1) var(--space-2); font-size: var(--text-xs); font-family: var(--font-sans); color: var(--text-secondary); background: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--border-radius-sm); cursor: pointer; transition: background 0.15s, color 0.15s; }',
      '.sd-csv-btn:hover { color: var(--text-primary); background: var(--bg-surface-inset); }',
      '.sd-csv-btn-copied { background: var(--color-success-bg) !important; color: var(--color-success-text) !important; border-color: var(--color-success) !important; }',
      '.sd-toggle-spacer { width: 24px; min-width: 24px; }',
      '.sd-legend { display: flex; gap: var(--space-4); flex-wrap: wrap; margin-bottom: var(--space-3); font-family: var(--font-sans); font-size: var(--text-xs); color: var(--text-secondary); }',
      '.sd-legend-item { display: inline-flex; align-items: center; gap: var(--space-1); }',
      '.sd-legend-swatch { display: inline-block; width: 12px; height: 12px; border-radius: 2px; }',
    ].join('\n');
    document.head.appendChild(style);
  }

  // ── 2. Table Builders — Read-Only ──

  function buildSortIndicator(colId, state) {
    var span = document.createElement('span');
    span.className = 'sd-sort-indicator';
    if (state.sortColumn === colId) {
      span.textContent = state.sortDirection === 'asc' ? '\u25B2' : '\u25BC';
    } else {
      span.textContent = '\u2195';
    }
    return span;
  }

  function buildExpandToggle(rowId, hasChildren, isExpanded, rerenderFn) {
    if (!hasChildren) {
      var spacer = document.createElement('span');
      spacer.className = 'sd-toggle-spacer';
      spacer.innerHTML = '&nbsp;';
      return spacer;
    }
    var btn = document.createElement('button');
    btn.className = 'sd-expand-toggle';
    btn.textContent = isExpanded ? '\u25BC' : '\u25B6';
    btn.addEventListener('click', function () {
      rerenderFn(function (state) {
        if (state.expandedRows.has(rowId)) {
          state.expandedRows.delete(rowId);
        } else {
          state.expandedRows.add(rowId);
        }
      });
    });
    return btn;
  }

  function buildDecisionToggle(key, state, rerenderFn, opts) {
    var wrapper = document.createElement('span');
    wrapper.className = 'sd-decision-toggle';
    var currentDecision = state.decisions[key] || 'accept';

    var acceptBtn = document.createElement('button');
    acceptBtn.textContent = '\u2713';
    acceptBtn.title = opts.acceptTitle || 'Accept';
    if (currentDecision === 'accept') acceptBtn.classList.add('sd-decision-accept');
    acceptBtn.addEventListener('click', function (e) {
      e.stopPropagation();
      state.decisions[key] = 'accept';
      rerenderFn();
    });

    var rejectBtn = document.createElement('button');
    rejectBtn.textContent = '\u2717';
    rejectBtn.title = opts.rejectTitle || 'Reject';
    if (currentDecision === 'reject') rejectBtn.classList.add('sd-decision-reject');
    rejectBtn.addEventListener('click', function (e) {
      e.stopPropagation();
      state.decisions[key] = 'reject';
      rerenderFn();
    });

    wrapper.appendChild(acceptBtn);
    wrapper.appendChild(rejectBtn);
    return wrapper;
  }

  function buildTableHeader(columns, state, reviewRequired, rerenderFn) {
    var thead = document.createElement('thead');
    var tr = document.createElement('tr');

    // Expand toggle spacer column
    var spacerTh = document.createElement('th');
    spacerTh.className = 'sd-th sd-toggle-spacer';
    tr.appendChild(spacerTh);

    columns.forEach(function (col) {
      var th = document.createElement('th');
      th.className = 'sd-th';

      if (reviewRequired) {
        if (col.change === 'add') th.classList.add('sd-col-add');
        if (col.change === 'delete') th.classList.add('sd-col-delete');

        // Check if column is rejected
        var colDecisionKey = 'col:' + col.id;
        if (state.decisions[colDecisionKey] === 'reject') {
          th.classList.add('sd-col-rejected');
        }
      }

      var nameSpan = document.createElement('span');
      nameSpan.textContent = col.name;
      th.appendChild(nameSpan);
      th.appendChild(buildSortIndicator(col.id, state));

      // Sort click handler
      th.addEventListener('click', function () {
        rerenderFn(function (s) {
          if (s.sortColumn === col.id) {
            if (s.sortDirection === 'asc') {
              s.sortDirection = 'desc';
            } else if (s.sortDirection === 'desc') {
              s.sortColumn = null;
              s.sortDirection = null;
            }
          } else {
            s.sortColumn = col.id;
            s.sortDirection = 'asc';
          }
        });
      });

      // Column decision toggle in review mode (for added or deleted columns)
      if (reviewRequired && (col.change === 'add' || col.change === 'delete')) {
        th.appendChild(buildDecisionToggle('col:' + col.id, state, rerenderFn, { acceptTitle: 'Accept column', rejectTitle: 'Reject column' }));
      }

      tr.appendChild(th);
    });

    // Decision column header if review mode
    if (reviewRequired) {
      var decTh = document.createElement('th');
      decTh.className = 'sd-th';
      decTh.textContent = 'Decision';
      tr.appendChild(decTh);
    }

    thead.appendChild(tr);
    return thead;
  }

  function buildTableBody(flatRows, columns, state, reviewRequired, rerenderFn) {
    var tbody = document.createElement('tbody');

    flatRows.forEach(function (entry) {
      var row = entry.row;
      var depth = entry.depth;
      var tr = document.createElement('tr');

      // Check if row is rejected
      if (state.decisions[row.id] === 'reject') {
        tr.classList.add('sd-row-rejected');
      }

      // Expand toggle cell
      var toggleTd = document.createElement('td');
      toggleTd.className = 'sd-td sd-toggle-spacer';
      var hasChildren = row.children && row.children.length > 0;
      var isExpanded = state.expandedRows.has(row.id);
      toggleTd.appendChild(buildExpandToggle(row.id, hasChildren, isExpanded, rerenderFn));
      tr.appendChild(toggleTd);

      columns.forEach(function (col, colIndex) {
        var td = document.createElement('td');
        td.className = 'sd-td';

        // Depth indentation on first cell
        if (colIndex === 0) {
          var depthClass = 'sd-depth-' + Math.min(depth, 5);
          td.classList.add(depthClass);
        }

        if (reviewRequired) {
          // Cell change styling
          var change = getCellChange(row, col.id);
          if (change === 'add') td.classList.add('sd-cell-add');
          if (change === 'delete') td.classList.add('sd-cell-delete');
          if (change === 'update') td.classList.add('sd-cell-update');

          // Check for user modifications
          var modKey = row.id + '.' + col.id;
          if (state.modifications[modKey]) {
            td.classList.add('sd-cell-edited');
          }

          // Column rejected styling
          var colDecisionKey = 'col:' + col.id;
          if (state.decisions[colDecisionKey] === 'reject') {
            td.classList.add('sd-col-rejected');
          }
        }

        var value = state.modifications[modKey]
          ? JSON.parse(state.modifications[modKey]).value
          : getCellValue(row, col.id);
        td.textContent = value;

        // Cell editor on double-click (review mode only)
        if (reviewRequired) {
          td.addEventListener('dblclick', function () {
            buildCellEditor(td, row.id, col.id, value, state, rerenderFn);
          });
          td.style.cursor = 'text';
        }

        tr.appendChild(td);
      });

      // Row decision toggle in review mode
      if (reviewRequired) {
        var decTd = document.createElement('td');
        decTd.className = 'sd-td';
        var hasChange = columns.some(function (col) {
          return getCellChange(row, col.id) != null;
        });
        if (hasChange) {
          decTd.appendChild(buildDecisionToggle(row.id, state, rerenderFn, { acceptTitle: 'Accept', rejectTitle: 'Reject' }));
        }
        tr.appendChild(decTd);
      }

      tbody.appendChild(tr);
    });

    return tbody;
  }

  function exportTableCsv(tableData, state) {
    var columns = tableData.columns;

    function escapeCsv(val) {
      var s = String(val == null ? '' : val);
      if (s.indexOf(',') !== -1 || s.indexOf('"') !== -1 || s.indexOf('\n') !== -1) {
        return '"' + s.replace(/"/g, '""') + '"';
      }
      return s;
    }

    function collectRows(rows, depth) {
      var result = [];
      if (!rows) return result;
      rows.forEach(function (row) {
        var cells = columns.map(function (col) {
          var modKey = row.id + '.' + col.id;
          if (state.modifications[modKey]) {
            return JSON.parse(state.modifications[modKey]).value;
          }
          return getCellValue(row, col.id);
        });
        result.push(cells);
        if (row.children && row.children.length > 0) {
          result = result.concat(collectRows(row.children, depth + 1));
        }
      });
      return result;
    }

    var header = columns.map(function (col) { return escapeCsv(col.name); });
    var rows = collectRows(tableData.rows, 0);
    var lines = [header.join(',')];
    rows.forEach(function (cells) {
      lines.push(cells.map(escapeCsv).join(','));
    });

    var csv = lines.join('\n');
    var fileName = (tableData.name || tableData.id || 'table') + '.csv';

    // Use Tauri IPC save_file command (native save dialog)
    if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
      return window.__TAURI__.core.invoke('save_file', {
        filename: fileName,
        content: csv
      });
    }

    // Fallback for non-Tauri environments: blob download
    var blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = fileName;
    a.style.display = 'none';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  function buildTableContainer(tableData, state, reviewRequired, onDecision) {
    var container = document.createElement('div');
    container.className = 'sd-container';

    // Table header row with name and CSV download
    var tableHeader = document.createElement('div');
    tableHeader.className = 'sd-table-header';

    if (tableData.name) {
      var nameEl = document.createElement('h3');
      nameEl.className = 'sd-table-name';
      nameEl.textContent = tableData.name;
      tableHeader.appendChild(nameEl);
    }

    var csvBtn = document.createElement('button');
    csvBtn.className = 'sd-csv-btn';
    csvBtn.textContent = 'CSV';
    csvBtn.title = 'Download table as CSV';
    csvBtn.addEventListener('click', function () {
      var result = exportTableCsv(tableData, state);
      if (result && typeof result.then === 'function') {
        result.then(function (saved) {
          if (saved) {
            var orig = csvBtn.textContent;
            csvBtn.textContent = 'Saved!';
            csvBtn.classList.add('sd-csv-btn-copied');
            setTimeout(function () {
              csvBtn.textContent = orig;
              csvBtn.classList.remove('sd-csv-btn-copied');
            }, 1500);
          }
        });
      }
    });
    tableHeader.appendChild(csvBtn);

    container.appendChild(tableHeader);

    // Per-table Accept All / Reject All (review mode only)
    if (reviewRequired) {
      var tableActions = document.createElement('div');
      tableActions.style.cssText = 'display: flex; gap: var(--space-2); margin-bottom: var(--space-2);';

      var tableAcceptAll = document.createElement('button');
      tableAcceptAll.textContent = 'Accept All';
      tableAcceptAll.style.cssText = 'padding: var(--space-1) var(--space-2); border-radius: var(--border-radius-sm); border: 1px solid var(--color-success); background: var(--color-success-bg); color: var(--color-success-text); cursor: pointer; font-size: var(--text-xs);';
      tableAcceptAll.addEventListener('click', function () {
        var tempStates = {};
        tempStates[tableData.id] = state;
        applyBulkDecision([tableData], tempStates, 'accept');
        renderTableContent();
      });

      var tableRejectAll = document.createElement('button');
      tableRejectAll.textContent = 'Reject All';
      tableRejectAll.style.cssText = 'padding: var(--space-1) var(--space-2); border-radius: var(--border-radius-sm); border: 1px solid var(--color-error); background: var(--color-error-bg); color: var(--color-error-text); cursor: pointer; font-size: var(--text-xs);';
      tableRejectAll.addEventListener('click', function () {
        var tempStates = {};
        tempStates[tableData.id] = state;
        applyBulkDecision([tableData], tempStates, 'reject');
        renderTableContent();
      });

      tableActions.appendChild(tableAcceptAll);
      tableActions.appendChild(tableRejectAll);
      container.appendChild(tableActions);
    }

    // Filter input
    var filter = document.createElement('input');
    filter.className = 'sd-filter';
    filter.type = 'text';
    filter.placeholder = 'Filter rows\u2026';
    filter.value = state.filterText;
    filter.addEventListener('input', function (e) {
      state.filterText = e.target.value;
      renderTableContent();
    });
    container.appendChild(filter);

    var table = document.createElement('table');
    table.className = 'sd-table';
    container.appendChild(table);

    function renderTableContent() {
      table.innerHTML = '';

      var rerenderFn = function (mutator) {
        if (mutator) mutator(state);
        renderTableContent();
      };

      // Process rows: filter, then sort
      var rows = tableData.rows;
      if (state.filterText) {
        rows = filterRows(JSON.parse(JSON.stringify(rows)), tableData.columns, state.filterText);
      }
      if (state.sortColumn && state.sortDirection) {
        rows = sortRows(JSON.parse(JSON.stringify(rows)), state.sortColumn, state.sortDirection);
      }

      var flatRows = flattenRows(rows, 0, state.expandedRows);

      table.appendChild(buildTableHeader(tableData.columns, state, reviewRequired, rerenderFn));
      table.appendChild(buildTableBody(flatRows, tableData.columns, state, reviewRequired, rerenderFn));
    }

    renderTableContent();

    // Expose rerender so global buttons can trigger it
    container.__rerender = renderTableContent;

    return container;
  }

  // ── 3. Table Builders — Review Mode ──

  function buildCellEditor(td, rowId, colId, currentValue, state, rerenderFn) {
    var input = document.createElement('input');
    input.className = 'sd-cell-editor';
    input.type = 'text';
    input.value = currentValue;
    td.innerHTML = '';
    td.appendChild(input);
    input.focus();
    input.select();

    function commit() {
      var newValue = input.value;
      if (newValue !== String(currentValue)) {
        var modKey = rowId + '.' + colId;
        state.modifications[modKey] = JSON.stringify({ value: newValue, user_edited: true });
      }
      rerenderFn();
    }

    input.addEventListener('blur', commit);
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') {
        e.preventDefault();
        input.blur();
      }
      if (e.key === 'Escape') {
        input.removeEventListener('blur', commit);
        rerenderFn();
      }
    });
  }

  function buildSubmitBar(tables, states, tableContainers, onDecision) {
    var bar = document.createElement('div');
    bar.className = 'sd-submit-bar';

    function rerenderAllTables() {
      tableContainers.forEach(function (tc) {
        if (tc.__rerender) tc.__rerender();
      });
    }

    var acceptAllBtn = document.createElement('button');
    acceptAllBtn.textContent = 'Accept All';
    acceptAllBtn.style.cssText = 'padding: var(--space-2) var(--space-3); border-radius: var(--border-radius-sm); border: 1px solid var(--color-success); background: var(--color-success-bg); color: var(--color-success-text); cursor: pointer; font-size: var(--text-small);';
    acceptAllBtn.addEventListener('click', function () {
      applyBulkDecision(tables, states, 'accept');
      rerenderAllTables();
    });

    var rejectAllBtn = document.createElement('button');
    rejectAllBtn.textContent = 'Reject All';
    rejectAllBtn.style.cssText = 'padding: var(--space-2) var(--space-3); border-radius: var(--border-radius-sm); border: 1px solid var(--color-error); background: var(--color-error-bg); color: var(--color-error-text); cursor: pointer; font-size: var(--text-small);';
    rejectAllBtn.addEventListener('click', function () {
      applyBulkDecision(tables, states, 'reject');
      rerenderAllTables();
    });

    var submitBtn = document.createElement('button');
    submitBtn.textContent = 'Submit Decisions';
    submitBtn.style.cssText = 'padding: var(--space-2) var(--space-4); border-radius: var(--border-radius-sm); border: 1px solid var(--color-info); background: var(--color-info); color: white; cursor: pointer; font-size: var(--text-small); font-weight: var(--weight-semibold);';
    submitBtn.addEventListener('click', function () {
      var payload = buildDecisionPayload(tables, states);
      if (onDecision) onDecision(payload);
    });

    bar.appendChild(acceptAllBtn);
    bar.appendChild(rejectAllBtn);
    bar.appendChild(submitBtn);
    return bar;
  }

  // ── 4. Legend ──

  function buildLegend(data, reviewRequired) {
    // Legend is only meaningful in review mode
    if (!reviewRequired) return null;

    // Detect which change types are present
    var hasAdd = false, hasDelete = false, hasUpdate = false;
    (data.tables || []).forEach(function (t) {
      t.columns.forEach(function (c) {
        if (c.change === 'add') hasAdd = true;
        if (c.change === 'delete') hasDelete = true;
      });
      function scanRows(rows) {
        if (!rows) return;
        rows.forEach(function (r) {
          if (r.cells) {
            Object.keys(r.cells).forEach(function (k) {
              var ch = r.cells[k].change;
              if (ch === 'add') hasAdd = true;
              if (ch === 'delete') hasDelete = true;
              if (ch === 'update') hasUpdate = true;
            });
          }
          if (r.children) scanRows(r.children);
        });
      }
      scanRows(t.rows);
    });

    // Only show legend if there are changes
    if (!hasAdd && !hasDelete && !hasUpdate && !reviewRequired) return null;

    var legend = document.createElement('div');
    legend.className = 'sd-legend';

    var items = [];
    if (hasAdd) items.push({ label: 'Added', bg: 'var(--color-success-bg)', border: 'var(--color-success)' });
    if (hasUpdate) items.push({ label: 'Modified', bg: 'var(--color-warning-bg)', border: 'var(--color-warning)' });
    if (hasDelete) items.push({ label: 'Deleted', bg: 'var(--color-error-bg)', border: 'var(--color-error)' });
    if (reviewRequired) items.push({ label: 'User edited', bg: 'var(--color-info-bg)', border: 'var(--color-info)' });

    items.forEach(function (item) {
      var el = document.createElement('span');
      el.className = 'sd-legend-item';
      var swatch = document.createElement('span');
      swatch.className = 'sd-legend-swatch';
      swatch.style.background = item.bg;
      swatch.style.border = '1px solid ' + item.border;
      el.appendChild(swatch);
      var label = document.createElement('span');
      label.textContent = item.label;
      el.appendChild(label);
      legend.appendChild(el);
    });

    return legend;
  }

  // ── 5. Orchestrator ──

  function renderStructuredData(container, data, meta, toolArgs, reviewRequired, onDecision) {
    container.innerHTML = '';
    injectStyles();

    if (!data || !data.tables || !data.tables.length) {
      var empty = document.createElement('div');
      empty.className = 'sd-empty';
      empty.textContent = 'No tables to display';
      container.appendChild(empty);
      return;
    }

    if (data.title) {
      var titleEl = document.createElement('h1');
      titleEl.className = 'sd-title';
      titleEl.textContent = data.title;
      container.appendChild(titleEl);
    }

    var legend = buildLegend(data, reviewRequired);
    if (legend) container.appendChild(legend);

    var states = {};
    var tableContainers = [];
    data.tables.forEach(function (tableData) {
      states[tableData.id] = createTableState(tableData);
      var tableContainer = buildTableContainer(tableData, states[tableData.id], reviewRequired, onDecision);
      tableContainers.push(tableContainer);
      container.appendChild(tableContainer);
    });

    if (reviewRequired && onDecision) {
      var submitBar = buildSubmitBar(data.tables, states, tableContainers, onDecision);
      container.appendChild(submitBar);
    }
  }

  window.__renderers.structured_data = renderStructuredData;
})();
