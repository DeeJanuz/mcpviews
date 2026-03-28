// @ts-nocheck
/* Rich content renderer — renders arbitrary markdown + mermaid + citations
 *
 * Data shape:
 * {
 *   title: "Optional Heading",        // Optional
 *   body: "## Markdown content\n\n```mermaid\ngraph LR\n  A-->B\n```\n\nMore text...",  // Required
 *   citations: { ... }                // Optional — same shape as search_results citations
 * }
 *
 * Also handles:
 * - Plain string input (treated as { body: data })
 * - Unknown data with no body/title (rendered as JSON fallback)
 */

(function () {
  'use strict';

  window.__renderers = window.__renderers || {};

  // ── Citation type map from legacy markers ──
  var CITE_TYPE_MAP = {
    documents: 'doc',
    code: 'code',
    dataGovernance: 'dg',
    data_governance: 'dg',
    api: 'api',
    knowledgeDex: 'kdex',
    knowledge_dex: 'kdex',
    dataLake: 'dl',
    data_lake: 'dl',
  };

  /**
   * Build a flat lookup map: { "doc:1": citationData, "code:2": citationData, ... }
   */
  function buildCitationMap(citations) {
    var map = {};
    if (!citations || typeof citations !== 'object') return map;

    Object.keys(citations).forEach(function (key) {
      var type = CITE_TYPE_MAP[key] || key;
      var items = citations[key];
      if (!Array.isArray(items)) return;
      items.forEach(function (item) {
        var idx = item.index != null ? item.index : item.number;
        if (idx != null) {
          map[type + ':' + idx] = item;
        }
      });
    });
    return map;
  }

  // ── Main renderer ──

  window.__renderers.rich_content = function renderRichContent(container, data, meta, toolArgs, reviewRequired, onDecision) {
    container.innerHTML = '';
    var utils = window.__companionUtils;

    // Normalize input: plain string → { body: data }
    if (typeof data === 'string') {
      data = { body: data };
    }

    // Fallback: if data has neither body nor title, render as JSON
    if (!data || (typeof data === 'object' && !data.body && !data.title)) {
      var pre = document.createElement('pre');
      pre.className = 'md-codeblock';
      pre.style.whiteSpace = 'pre-wrap';
      pre.style.wordBreak = 'break-word';
      pre.textContent = JSON.stringify(data, null, 2);
      container.appendChild(pre);
      return;
    }

    // Title + view toggle
    var headerRow = document.createElement('div');
    headerRow.className = 'rc-header';

    if (data.title) {
      var h1 = document.createElement('h1');
      h1.className = 'rc-title';
      h1.textContent = data.title;
      headerRow.appendChild(h1);
    }

    if (data.body) {
      var toggleBtn = document.createElement('button');
      toggleBtn.className = 'rc-view-toggle';
      toggleBtn.textContent = 'Markdown';
      toggleBtn.title = 'View raw markdown';
      headerRow.appendChild(toggleBtn);
    }

    container.appendChild(headerRow);

    // Body
    if (data.body) {
      var hasCitations = data.citations && typeof data.citations === 'object' && Object.keys(data.citations).length > 0;
      var contentEl;

      if (hasCitations) {
        contentEl = utils.renderMarkdownWithCitations(data.body);
      } else {
        contentEl = utils.renderMarkdown(data.body);
      }

      // Raw markdown view (hidden by default)
      var rawEl = document.createElement('pre');
      rawEl.className = 'rc-raw-markdown';
      rawEl.style.display = 'none';
      var rawCode = document.createElement('code');
      rawCode.textContent = data.body;
      rawEl.appendChild(rawCode);

      if (contentEl instanceof HTMLElement) {
        container.appendChild(contentEl);
        container.appendChild(rawEl);

        // Render mermaid diagrams
        utils.renderMermaidBlocks(contentEl);

        // Toggle between rendered and raw
        var showingRaw = false;
        toggleBtn.addEventListener('click', function () {
          showingRaw = !showingRaw;
          if (showingRaw) {
            contentEl.style.display = 'none';
            rawEl.style.display = '';
            toggleBtn.textContent = 'Rendered';
            toggleBtn.title = 'View rendered content';
            toggleBtn.classList.add('rc-view-toggle-active');
          } else {
            contentEl.style.display = '';
            rawEl.style.display = 'none';
            toggleBtn.textContent = 'Markdown';
            toggleBtn.title = 'View raw markdown';
            toggleBtn.classList.remove('rc-view-toggle-active');
          }
        });
      }

      // Wire up citation clicks
      if (hasCitations) {
        var citationMap = buildCitationMap(data.citations);

        container.addEventListener('click', function (e) {
          var citeEl = e.target.closest('[data-cite-type]');
          if (!citeEl) return;

          var type = citeEl.getAttribute('data-cite-type');
          var index = citeEl.getAttribute('data-cite-index');
          var key = type + ':' + index;
          var citationData = citationMap[key];

          if (citationData && utils.openCitationPanel) {
            e.stopPropagation();
            utils.openCitationPanel(type, citationData);
          }
        });
      }
    }
  };
})();
