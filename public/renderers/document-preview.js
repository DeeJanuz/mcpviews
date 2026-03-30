// @ts-nocheck
/* Document preview renderer — document_preview (read-only) & document_diff (edit review) */

(function () {
  'use strict';

  window.__renderers = window.__renderers || {};

  /* ────────────────────────────────────────────────
   * renderSingleDocument — reusable single-doc view with links
   * ──────────────────────────────────────────────── */
  function renderSingleDocument(container, doc, utils) {
    var titleRow = document.createElement('div');
    titleRow.className = 'doc-title-row';
    var title = document.createElement('h1');
    title.className = 'doc-title';
    title.textContent = doc.title || 'Untitled';
    titleRow.appendChild(title);
    container.appendChild(titleRow);

    var metaRow = document.createElement('div');
    metaRow.className = 'doc-meta-row';
    if (doc.status) metaRow.appendChild(utils.createStatusBadge(doc.status));
    if (doc.folder_name) {
      var folder = document.createElement('span');
      folder.className = 'doc-folder';
      folder.textContent = '\uD83D\uDCC1 ' + doc.folder_name;
      metaRow.appendChild(folder);
    }
    container.appendChild(metaRow);

    if (doc.content) {
      var contentContainer = document.createElement('div');
      contentContainer.className = 'doc-content-container';
      var md = utils.renderMarkdown(doc.content);
      if (md instanceof HTMLElement) contentContainer.appendChild(md);
      container.appendChild(contentContainer);
      utils.renderMermaidBlocks(contentContainer);
    }

    // Render links if present
    if (doc.links && doc.links.length > 0) {
      var linksSection = document.createElement('div');
      linksSection.className = 'doc-links-section';
      var linksTitle = document.createElement('h3');
      linksTitle.className = 'doc-links-heading';
      linksTitle.textContent = 'Linked Documents';
      linksSection.appendChild(linksTitle);

      doc.links.forEach(function (link) {
        var linkEl = document.createElement('div');
        linkEl.className = 'doc-link-card';
        linkEl.textContent = link.title || link.target_document_id || 'Linked Document';
        // Click to navigate to linked document
        var targetId = link.target_document_id || link.id;
        if (targetId && utils.isProxyConfigured()) {
          linkEl.style.cursor = 'pointer';
          linkEl.addEventListener('click', function () {
            linkEl.style.opacity = '0.6';
            utils.companionFetch('get_document', { document_id: targetId, include_links: true })
              .then(function (result) {
                var linkedDoc = null;
                if (result && result.content && result.content[0]) {
                  try { linkedDoc = JSON.parse(result.content[0].text); } catch (e) {}
                }
                if (linkedDoc && linkedDoc.data) {
                  // Save current doc for back navigation
                  var previousDoc = doc;
                  container.innerHTML = '';
                  var backBtn = utils.createButton('\u2190 Back', {
                    bg: 'var(--bg-surface-inset)', color: 'var(--text-primary)',
                    onclick: function () {
                      container.innerHTML = '';
                      renderSingleDocument(container, previousDoc, utils);
                    }
                  });
                  backBtn.style.marginBottom = '16px';
                  container.appendChild(backBtn);
                  renderSingleDocument(container, linkedDoc.data, utils);
                }
              })
              .catch(function () { linkEl.style.opacity = '1'; });
          });
        }

        linksSection.appendChild(linkEl);
      });

      container.appendChild(linksSection);
    }
  }

  /* ────────────────────────────────────────────────
   * document_preview — read-only for get_document / list_documents
   * ──────────────────────────────────────────────── */
  function renderDocumentPreview(container, data, meta, toolArgs, onDecision) {
    var utils = window.__companionUtils;
    container.innerHTML = '';

    // Handle list_documents (array of docs)
    var raw = (data && data.data) || data;
    var docs = Array.isArray(raw) ? raw : [raw || {}];

    if (docs.length > 1) {
      // Card list for multiple documents
      var header = document.createElement('div');
      header.className = 'doc-list-header';
      header.appendChild(utils.createBadge(docs.length + ' documents', 'var(--bg-surface-inset)', 'var(--text-primary)'));
      container.appendChild(header);

      docs.forEach(function (doc) {
        var card = document.createElement('div');
        card.className = 'doc-list-card';

        var titleRow = document.createElement('div');
        titleRow.className = 'doc-card-title-row';
        var title = document.createElement('span');
        title.className = 'doc-card-title';
        title.textContent = doc.title || 'Untitled';
        titleRow.appendChild(title);
        if (doc.status) titleRow.appendChild(utils.createStatusBadge(doc.status));
        card.appendChild(titleRow);

        if (doc.folder_name) {
          var folder = document.createElement('div');
          folder.className = 'doc-folder';
          folder.style.marginBottom = '8px';
          folder.textContent = '\uD83D\uDCC1 ' + doc.folder_name;
          card.appendChild(folder);
        }

        if (doc.content) {
          var preview = document.createElement('div');
          preview.className = 'doc-card-preview';
          preview.textContent = utils.truncate(doc.content, 200);
          card.appendChild(preview);
        }

        // Set cursor and click handler based on proxy availability
        if (doc.id) {
          utils.checkProxyStatus().then(function (status) {
            if (status && status.configured) {
              card.style.cursor = 'pointer';
            } else {
              card.style.cursor = 'default';
            }
          });

          card.addEventListener('click', function () {
            if (!utils.isProxyConfigured()) return;
            if (!doc.id) return;

            // Show loading state
            card.style.opacity = '0.6';
            card.style.pointerEvents = 'none';
            var loadingText = document.createElement('div');
            loadingText.className = 'doc-loading-text';
            loadingText.textContent = 'Loading...';
            card.appendChild(loadingText);

            utils.companionFetch('get_document', { document_id: doc.id, include_links: true })
              .then(function (result) {
                // Parse the result — MCP tools return { content: [{ type: 'text', text: '...' }] }
                var docData = null;
                if (result && result.content && result.content[0]) {
                  try { docData = JSON.parse(result.content[0].text); } catch (e) {}
                }
                if (!docData || !docData.data) {
                  throw new Error('Invalid response');
                }

                // Clear container and render single doc with back button
                container.innerHTML = '';

                var backBtn = utils.createButton('\u2190 Back to list', {
                  bg: 'var(--bg-surface-inset)',
                  color: 'var(--text-primary)',
                  onclick: function () {
                    renderDocumentPreview(container, data, meta, toolArgs, onDecision);
                  }
                });
                backBtn.style.marginBottom = '16px';
                container.appendChild(backBtn);

                // Render the fetched document using single-doc view
                renderSingleDocument(container, docData.data, utils);
              })
              .catch(function () {
                // Show error inline
                card.style.opacity = '1';
                card.style.pointerEvents = 'auto';
                if (loadingText.parentNode) loadingText.parentNode.removeChild(loadingText);
                var errEl = document.createElement('div');
                errEl.className = 'doc-error-text';
                errEl.textContent = 'Failed to load document';
                card.appendChild(errEl);
                setTimeout(function () { if (errEl.parentNode) errEl.parentNode.removeChild(errEl); }, 3000);
              });
          });
        }

        container.appendChild(card);
      });
      return;
    }

    // Single document view
    var doc = docs[0];
    renderSingleDocument(container, doc, utils);
  }

  /* ────────────────────────────────────────────────
   * document_diff — edit review for write_document
   * Google Docs-style right margin annotation layout
   * ──────────────────────────────────────────────── */

  /* ── State factory ── */
  function createDiffState() {
    return {
      decisions: {},
      comments: {},
      activeChipId: null,
      sidebarCards: {},
      chipElements: {},
      navItemElements: {}
    };
  }

  /* ── CSS class for operation-type dot color ── */
  function getDotClass(operationType) {
    if (operationType === 'insert') return 'diff-nav-dot-insert';
    if (operationType === 'delete') return 'diff-nav-dot-delete';
    return 'diff-nav-dot-replace';
  }

  /* ── DOM builder: unchanged segment ── */
  function renderUnchangedSegment(content) {
    var span = document.createElement('div');
    span.className = 'op-unchanged';
    span.textContent = content;
    return span;
  }

  /* ── DOM builder: operation region in document panel ── */
  function renderOperationRegion(seg, state) {
    var typeClass = 'op-region-' + (seg.operationType || 'replace');
    var region = document.createElement('div');
    region.id = 'op-' + seg.operationId;
    region.className = 'op-region ' + typeClass;

    // Original text (strikethrough red) — for replace and delete
    if (seg.operationType === 'replace' || seg.operationType === 'delete') {
      var original = document.createElement('div');
      original.className = 'op-original';
      original.setAttribute('data-role', 'original');
      original.textContent = seg.originalText;
      region.appendChild(original);
    }

    // Replacement text (green) — for replace and insert
    if (seg.operationType === 'replace' || seg.operationType === 'insert') {
      var replacement = document.createElement('div');
      replacement.className = 'op-replacement';
      replacement.setAttribute('data-role', 'replacement');
      replacement.textContent = seg.replacementText;
      region.appendChild(replacement);
    }

    // Click region to activate
    region.addEventListener('click', function () {
      setActiveOp(seg.operationId, state);
    });

    return region;
  }

  /* ── DOM builder: sidebar annotation card ── */
  function renderSidebarCard(seg, state, reviewRequired, refreshUI) {
    var card = document.createElement('div');
    card.className = 'diff-card diff-card-pending';
    card.setAttribute('data-op-id', seg.operationId);

    // Description
    var descEl = document.createElement('div');
    descEl.className = 'diff-card-desc';
    descEl.textContent = seg.description || (seg.operationType || 'edit');
    card.appendChild(descEl);

    // Decision label (hidden initially)
    var decisionLabel = document.createElement('div');
    decisionLabel.className = 'diff-card-decision';
    decisionLabel.setAttribute('data-role', 'decision-label');
    decisionLabel.style.display = 'none';
    card.appendChild(decisionLabel);

    // Buttons row
    if (reviewRequired) {
      var btnRow = document.createElement('div');
      btnRow.className = 'diff-card-buttons';
      btnRow.setAttribute('data-role', 'buttons');

      var acceptBtn = document.createElement('button');
      acceptBtn.className = 'diff-card-btn diff-card-btn-accept';
      acceptBtn.setAttribute('data-role', 'accept-btn');
      acceptBtn.textContent = '\u2713 Accept';
      acceptBtn.addEventListener('click', function (e) {
        e.stopPropagation();
        state.decisions[seg.operationId] = 'accepted';
        refreshUI();
      });

      var rejectBtn = document.createElement('button');
      rejectBtn.className = 'diff-card-btn diff-card-btn-reject';
      rejectBtn.setAttribute('data-role', 'reject-btn');
      rejectBtn.textContent = '\u2717 Reject';
      rejectBtn.addEventListener('click', function (e) {
        e.stopPropagation();
        state.decisions[seg.operationId] = 'rejected';
        refreshUI();
      });

      btnRow.appendChild(acceptBtn);
      btnRow.appendChild(rejectBtn);
      card.appendChild(btnRow);
    }

    // Comment textarea
    var commentArea = document.createElement('textarea');
    commentArea.className = 'diff-card-comment';
    commentArea.placeholder = 'Add a comment...';
    commentArea.addEventListener('input', function () {
      state.comments[seg.operationId] = commentArea.value;
    });
    commentArea.addEventListener('click', function (e) { e.stopPropagation(); });
    card.appendChild(commentArea);

    // Click card to scroll to operation + activate
    card.addEventListener('click', function () {
      setActiveOp(seg.operationId, state);
      var target = document.getElementById('op-' + seg.operationId);
      if (target) target.scrollIntoView({ behavior: 'smooth', block: 'center' });
    });

    return card;
  }

  /* ── DOM builder: sticky nav widget ── */
  function buildStickyNav(opSegments, state, utils, summaryBar, refreshUI) {
    var stickyNav = document.createElement('div');
    stickyNav.className = 'diff-nav-sticky';

    var navHeader = document.createElement('div');
    navHeader.className = 'diff-nav-header';
    navHeader.textContent = 'Operations';
    stickyNav.appendChild(navHeader);

    var navList = document.createElement('div');
    navList.className = 'diff-nav-list';

    for (var c = 0; c < opSegments.length; c++) {
      (function (seg) {
        var navItem = document.createElement('div');
        navItem.className = 'diff-nav-item';
        navItem.setAttribute('data-op-id', seg.operationId);

        var dot = document.createElement('span');
        dot.className = 'diff-nav-dot ' + getDotClass(seg.operationType);
        dot.setAttribute('data-role', 'nav-dot');
        navItem.appendChild(dot);

        var label = document.createElement('span');
        label.className = 'diff-nav-label';
        label.textContent = utils.truncate(seg.description || (seg.operationType || 'edit'), 32);
        label.title = seg.description || '';
        navItem.appendChild(label);

        state.chipElements[seg.operationId] = navItem;
        state.navItemElements[seg.operationId] = navItem;

        navItem.addEventListener('click', function () {
          setActiveOp(seg.operationId, state);
          var target = document.getElementById('op-' + seg.operationId);
          if (target) target.scrollIntoView({ behavior: 'smooth', block: 'center' });
        });

        navList.appendChild(navItem);
      })(opSegments[c]);
    }

    stickyNav.appendChild(navList);

    // Summary + submit row
    var summaryRow = document.createElement('div');
    summaryRow.className = 'diff-nav-summary-row';
    summaryRow.appendChild(summaryBar);

    stickyNav.appendChild(summaryRow);

    return { stickyNav: stickyNav, summaryRow: summaryRow };
  }

  /* ── DOM builder: document panel (left) ── */
  function buildDocumentPanel(segments, state) {
    var docPanel = document.createElement('div');
    docPanel.className = 'diff-doc-panel';

    for (var s = 0; s < segments.length; s++) {
      var seg = segments[s];
      if (seg.type === 'unchanged') {
        docPanel.appendChild(renderUnchangedSegment(seg.content));
      } else if (seg.type === 'operation') {
        docPanel.appendChild(renderOperationRegion(seg, state));
      }
    }

    return docPanel;
  }

  /* ── DOM builder: sidebar cards ── */
  function buildSidebarCards(opSegments, state, reviewRequired, refreshUI) {
    var sidebarInner = document.createElement('div');
    sidebarInner.className = 'diff-sidebar-inner';

    for (var i = 0; i < opSegments.length; i++) {
      (function (seg) {
        var card = renderSidebarCard(seg, state, reviewRequired, refreshUI);
        state.sidebarCards[seg.operationId] = card;
        sidebarInner.appendChild(card);
      })(opSegments[i]);
    }

    return sidebarInner;
  }

  /* ── State mutation: set active operation ── */
  function setActiveOp(opId, state) {
    if (state.activeChipId && state.navItemElements[state.activeChipId]) {
      state.navItemElements[state.activeChipId].classList.remove('diff-nav-item-active');
    }
    // Remove active from all sidebar cards
    for (var id in state.sidebarCards) {
      state.sidebarCards[id].classList.remove('diff-card-active');
    }
    state.activeChipId = opId;
    if (state.navItemElements[opId]) state.navItemElements[opId].classList.add('diff-nav-item-active');
    if (state.sidebarCards[opId]) state.sidebarCards[opId].classList.add('diff-card-active');
  }

  /* ── UI updater: render summary bar counts ── */
  function renderSummaryBar(summaryBar, opSegments, state) {
    var accepted = 0, rejected = 0, pending = 0;
    for (var i = 0; i < opSegments.length; i++) {
      var d = state.decisions[opSegments[i].operationId];
      if (d === 'accepted') accepted++;
      else if (d === 'rejected') rejected++;
      else pending++;
    }

    summaryBar.innerHTML = '';

    var totalSpan = document.createElement('span');
    totalSpan.style.color = 'var(--text-primary)';
    totalSpan.innerHTML = '<span class="count">' + opSegments.length + '</span> operations';
    summaryBar.appendChild(totalSpan);

    var acceptedSpan = document.createElement('span');
    acceptedSpan.className = 'accepted';
    acceptedSpan.innerHTML = '<span class="count">' + accepted + '</span> accepted';
    summaryBar.appendChild(acceptedSpan);

    var rejectedSpan = document.createElement('span');
    rejectedSpan.className = 'rejected';
    rejectedSpan.innerHTML = '<span class="count">' + rejected + '</span> rejected';
    summaryBar.appendChild(rejectedSpan);

    var pendingSpan = document.createElement('span');
    pendingSpan.className = 'pending';
    pendingSpan.innerHTML = '<span class="count">' + pending + '</span> pending';
    summaryBar.appendChild(pendingSpan);
  }

  /* ── UI updater: position sidebar cards to align with operation regions ── */
  function positionSidebarCards(opSegments, state, sidebarInner, twoCol) {
    var MIN_GAP = 8;
    var lastBottom = 0;

    // Account for sidebarInner offset within twoCol (sticky nav pushes it down)
    var innerRect = sidebarInner.getBoundingClientRect();
    var containerRect = twoCol.getBoundingClientRect();
    var innerOffset = innerRect.top - containerRect.top;

    for (var i = 0; i < opSegments.length; i++) {
      var seg = opSegments[i];
      var region = document.getElementById('op-' + seg.operationId);
      var card = state.sidebarCards[seg.operationId];
      if (!region || !card) continue;

      // Get the operation region's top relative to sidebarInner
      var regionRect = region.getBoundingClientRect();
      var desiredTop = regionRect.top - containerRect.top - innerOffset;

      // Ensure cards don't overlap
      var top = Math.max(desiredTop, lastBottom + MIN_GAP);
      card.style.position = 'absolute';
      card.style.top = top + 'px';
      card.style.left = '0';
      card.style.right = '0';

      lastBottom = top + card.offsetHeight;
    }

    // Set sidebar inner min-height to contain all cards
    if (lastBottom > 0) {
      sidebarInner.style.minHeight = (lastBottom + 16) + 'px';
    }
  }

  /* ── UI updater: refresh all UI after decision changes ── */
  function refreshAllUI(opSegments, state, summaryBar, sidebarInner, twoCol) {
    // Update summary bar
    renderSummaryBar(summaryBar, opSegments, state);

    // Update nav item states
    for (var opId in state.navItemElements) {
      var navItem = state.navItemElements[opId];
      var seg = null;
      for (var i = 0; i < opSegments.length; i++) {
        if (opSegments[i].operationId === opId) { seg = opSegments[i]; break; }
      }

      navItem.className = 'diff-nav-item';
      if (opId === state.activeChipId) navItem.classList.add('diff-nav-item-active');

      var dot = navItem.querySelector('[data-role="nav-dot"]');
      if (dot) {
        if (state.decisions[opId] === 'accepted') {
          navItem.classList.add('diff-nav-item-accepted');
          dot.textContent = '\u2713';
          dot.className = 'diff-nav-dot diff-nav-dot-accepted diff-nav-dot-decided';
          dot.style.background = '';
        } else if (state.decisions[opId] === 'rejected') {
          navItem.classList.add('diff-nav-item-rejected');
          dot.textContent = '\u2717';
          dot.className = 'diff-nav-dot diff-nav-dot-rejected diff-nav-dot-decided';
          dot.style.background = '';
        } else {
          dot.textContent = '';
          dot.className = 'diff-nav-dot ' + (seg ? getDotClass(seg.operationType) : 'diff-nav-dot-replace');
          dot.style.background = '';
        }
      }
    }

    // Update operation regions in document panel
    for (var i = 0; i < opSegments.length; i++) {
      var seg = opSegments[i];
      var region = document.getElementById('op-' + seg.operationId);
      if (!region) continue;

      var typeClass = 'op-region-' + (seg.operationType || 'replace');
      var originalEl = region.querySelector('[data-role="original"]');
      var replacementEl = region.querySelector('[data-role="replacement"]');

      if (state.decisions[seg.operationId] === 'accepted') {
        region.className = 'op-region-decided';
        if (originalEl) originalEl.style.display = 'none';
        if (replacementEl) {
          replacementEl.style.display = 'block';
          replacementEl.className = 'op-accepted-text';
        }
      } else if (state.decisions[seg.operationId] === 'rejected') {
        region.className = 'op-region-decided';
        if (originalEl) {
          originalEl.style.display = 'block';
          originalEl.className = 'op-rejected-text';
        }
        if (replacementEl) replacementEl.style.display = 'none';
      } else {
        region.className = 'op-region ' + typeClass;
        if (originalEl) {
          originalEl.style.display = 'block';
          originalEl.className = 'op-original';
        }
        if (replacementEl) {
          replacementEl.style.display = 'block';
          replacementEl.className = 'op-replacement';
        }
      }
    }

    // Update sidebar cards
    for (var j = 0; j < opSegments.length; j++) {
      var seg2 = opSegments[j];
      var card = state.sidebarCards[seg2.operationId];
      if (!card) continue;

      var decision = state.decisions[seg2.operationId];
      var decLabel = card.querySelector('[data-role="decision-label"]');
      var acceptBtn = card.querySelector('[data-role="accept-btn"]');
      var rejectBtn = card.querySelector('[data-role="reject-btn"]');

      if (decision === 'accepted') {
        card.className = 'diff-card diff-card-accepted';
        if (state.activeChipId === seg2.operationId) card.classList.add('diff-card-active');
        if (decLabel) {
          decLabel.style.display = 'block';
          decLabel.textContent = 'Accepted \u2713';
          decLabel.style.color = 'var(--color-success-text)';
        }
        if (acceptBtn) acceptBtn.style.display = 'none';
        if (rejectBtn) {
          rejectBtn.style.display = 'inline-flex';
          rejectBtn.textContent = '\u21A9 Undo';
        }
      } else if (decision === 'rejected') {
        card.className = 'diff-card diff-card-rejected';
        if (state.activeChipId === seg2.operationId) card.classList.add('diff-card-active');
        if (decLabel) {
          decLabel.style.display = 'block';
          decLabel.textContent = 'Rejected \u2717';
          decLabel.style.color = 'var(--color-error-text)';
        }
        if (rejectBtn) rejectBtn.style.display = 'none';
        if (acceptBtn) {
          acceptBtn.style.display = 'inline-flex';
          acceptBtn.textContent = '\u21A9 Undo';
        }
      } else {
        card.className = 'diff-card diff-card-pending';
        if (state.activeChipId === seg2.operationId) card.classList.add('diff-card-active');
        if (decLabel) decLabel.style.display = 'none';
        if (acceptBtn) {
          acceptBtn.style.display = 'inline-flex';
          acceptBtn.textContent = '\u2713 Accept';
        }
        if (rejectBtn) {
          rejectBtn.style.display = 'inline-flex';
          rejectBtn.textContent = '\u2717 Reject';
        }
      }
    }

    // Re-position sidebar cards after layout changes
    requestAnimationFrame(function () {
      positionSidebarCards(opSegments, state, sidebarInner, twoCol);
    });

    // Show submit if all decided
    var allDecided = opSegments.length > 0 && opSegments.every(function (s) {
      return state.decisions[s.operationId];
    });
    var sb = document.getElementById('submit-btn');
    if (sb) sb.style.display = allDecided ? 'inline-flex' : 'none';
  }

  /* ── Orchestrator ── */
  function renderDocumentDiff(container, data, meta, toolArgs, reviewRequired, onDecision) {
    var utils = window.__companionUtils;
    container.innerHTML = '';

    var doc = (data && data.data) || data || {};
    var originalContent = doc.content || '';
    var operations = (toolArgs && toolArgs.operations) || [];

    // Title
    var titleEl = document.createElement('h2');
    titleEl.className = 'doc-diff-title';
    titleEl.textContent = 'Edit: ' + (doc.title || (toolArgs && toolArgs.document_id) || 'Document');
    container.appendChild(titleEl);

    if (operations.length === 0) {
      var noOps = document.createElement('div');
      noOps.className = 'doc-no-ops';
      noOps.textContent = 'No operations to review';
      container.appendChild(noOps);
      return;
    }

    // Create state and compute segments
    var state = createDiffState();
    var segments = computeDocumentSegments(originalContent, operations);
    var opSegments = segments.filter(function (s) { return s.type === 'operation'; });

    // Summary bar element (shared between nav and refreshUI)
    var summaryBar = document.createElement('div');
    summaryBar.className = 'summary-bar diff-nav-summary';

    // Create refreshUI closure that binds state + DOM references
    var sidebarInner = null;
    var twoCol = document.createElement('div');
    twoCol.className = 'diff-two-col';

    var refreshUI = function () {
      refreshAllUI(opSegments, state, summaryBar, sidebarInner, twoCol);
    };

    // Bulk action buttons
    if (reviewRequired) {
      var bulkRow = document.createElement('div');
      bulkRow.className = 'doc-bulk-row';

      var acceptAllBtn = utils.createButton('Accept All', {
        bg: 'var(--color-success-bg)', color: 'var(--color-success-text)',
        onclick: function () {
          for (var i = 0; i < opSegments.length; i++) {
            state.decisions[opSegments[i].operationId] = 'accepted';
          }
          refreshUI();
        }
      });
      bulkRow.appendChild(acceptAllBtn);

      var rejectAllBtn = utils.createButton('Reject All', {
        bg: 'var(--color-error-bg)', color: 'var(--color-error-text)',
        onclick: function () {
          for (var i = 0; i < opSegments.length; i++) {
            state.decisions[opSegments[i].operationId] = 'rejected';
          }
          refreshUI();
        }
      });
      bulkRow.appendChild(rejectAllBtn);

      container.appendChild(bulkRow);
    }

    // Build document panel (left)
    var docPanel = buildDocumentPanel(segments, state);
    twoCol.appendChild(docPanel);

    // Build sidebar panel (right)
    var sidebarPanel = document.createElement('div');
    sidebarPanel.className = 'diff-sidebar-panel';

    // Build sticky nav
    var navResult = buildStickyNav(opSegments, state, utils, summaryBar, refreshUI);

    // Add submit button to summary row
    var submitBtn = utils.createButton('Submit', {
      bg: 'var(--accent-primary)', color: 'var(--text-inverse)',
      onclick: function () {
        var currentSessionId = meta && meta.sessionId;
        var commentPayload = {};
        for (var opId in state.comments) {
          if (state.comments[opId]) commentPayload[opId] = state.comments[opId];
        }
        onDecision({
          type: 'operation_decisions',
          sessionId: currentSessionId,
          decisions: state.decisions,
          comments: commentPayload
        });
      }
    });
    submitBtn.id = 'submit-btn';
    submitBtn.className = 'diff-nav-submit';
    submitBtn.style.display = 'none';
    navResult.summaryRow.appendChild(submitBtn);

    sidebarPanel.appendChild(navResult.stickyNav);

    // Build sidebar cards
    sidebarInner = buildSidebarCards(opSegments, state, reviewRequired, refreshUI);
    sidebarPanel.appendChild(sidebarInner);

    twoCol.appendChild(sidebarPanel);
    container.appendChild(twoCol);

    // Initial summary render
    renderSummaryBar(summaryBar, opSegments, state);

    // Position sidebar cards after layout
    requestAnimationFrame(function () {
      positionSidebarCards(opSegments, state, sidebarInner, twoCol);
    });
  }

  /* ────────────────────────────────────────────────
   * computeDocumentSegments — ported from lib/chat/diff-utils.ts
   * ──────────────────────────────────────────────── */
  function computeDocumentSegments(originalContent, operations) {
    if (!operations || operations.length === 0) {
      return [{ type: 'unchanged', content: originalContent }];
    }

    var positioned = operations
      .map(function (op) {
        var search = op.search_text || op.search || '';
        var pos = originalContent.indexOf(search);
        return { op: op, pos: pos, search: search };
      })
      .filter(function (item) { return item.pos !== -1; })
      .sort(function (a, b) { return a.pos - b.pos; });

    if (positioned.length === 0) {
      return [{ type: 'unchanged', content: originalContent }];
    }

    var segments = [];
    var cursor = 0;

    for (var i = 0; i < positioned.length; i++) {
      var item = positioned[i];
      var op = item.op;
      var pos = item.pos;
      var search = item.search;

      if (pos > cursor) {
        segments.push({ type: 'unchanged', content: originalContent.slice(cursor, pos) });
      }

      var opId = op.id || ('op-' + segments.length);
      segments.push({
        type: 'operation',
        operationId: opId,
        operationType: op.type || 'replace',
        description: op.description || '',
        originalText: search,
        replacementText: op.replace_text || op.replace || '',
      });

      cursor = pos + search.length;
    }

    if (cursor < originalContent.length) {
      segments.push({ type: 'unchanged', content: originalContent.slice(cursor) });
    }

    return segments;
  }

  /* ── Register renderers ── */
  window.__renderers['document_preview'] = renderDocumentPreview;
  window.__renderers['document_diff'] = renderDocumentDiff;
})();
