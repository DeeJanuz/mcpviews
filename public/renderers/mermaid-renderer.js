// @ts-nocheck
/* Mermaid diagram rendering — async placeholder rendering + expand modal with zoom/pan */

(function () {
  'use strict';

  var utils = window.__companionUtils;
  var _mermaidCounter = 0;

  /**
   * Post-processor: find .mermaid-placeholder elements, decode base64 source,
   * call mermaid.render() async, replace with SVG. On error show fallback.
   * Each rendered diagram gets a click handler to open the expand modal.
   */
  function renderMermaidBlocks(container) {
    if (typeof mermaid === 'undefined') return;
    var placeholders = container.querySelectorAll('.mermaid-placeholder');
    if (placeholders.length === 0) return;

    for (var i = 0; i < placeholders.length; i++) {
      (function (el) {
        var encoded = el.getAttribute('data-mermaid');
        if (!encoded) return;

        var source;
        try {
          source = decodeURIComponent(escape(atob(encoded)));
        } catch (e) {
          el.innerHTML = '<div class="mermaid-error"><span class="mermaid-error-label">Decode error</span></div>';
          return;
        }

        var id = 'mermaid-' + (++_mermaidCounter);
        var isDark = document.documentElement.getAttribute('data-theme') === 'dark';
        if (typeof mermaid !== 'undefined' && mermaid.initialize) {
          mermaid.initialize({ startOnLoad: false, theme: isDark ? 'dark' : 'default', securityLevel: 'loose', suppressErrorRendering: true });
        }
        mermaid.render(id, source)
          .then(function (result) {
            el.innerHTML = '';
            var rendered = document.createElement('div');
            rendered.className = 'mermaid-rendered';
            rendered.innerHTML = result.svg;
            rendered.setAttribute('data-mermaid-source', encoded);
            rendered.title = 'Click to expand';
            rendered.addEventListener('click', function () {
              openMermaidModal(rendered);
            });
            el.appendChild(rendered);
            el.classList.remove('mermaid-placeholder');
          })
          .catch(function (err) {
            el.innerHTML = '';
            var errWrap = document.createElement('div');
            errWrap.className = 'mermaid-error';
            var label = document.createElement('span');
            label.className = 'mermaid-error-label';
            label.textContent = 'Diagram error';
            errWrap.appendChild(label);
            var codeBlock = document.createElement('pre');
            codeBlock.className = 'md-codeblock';
            codeBlock.style.marginTop = '8px';
            var codeEl = document.createElement('code');
            codeEl.textContent = source;
            codeBlock.appendChild(codeEl);
            errWrap.appendChild(codeBlock);
            el.appendChild(errWrap);
          });
      })(placeholders[i]);
    }
  }

  // ── Expand modal state ──

  var _modal = null;
  var _modalBody = null;
  var _zoomLevel = 1;
  var _zoomLabel = null;
  var _svgWrap = null;
  var _naturalW = 0;
  var _naturalH = 0;

  function _updateZoom() {
    if (_svgWrap && _naturalW > 0 && _naturalH > 0) {
      var svg = _svgWrap.querySelector('svg');
      if (svg) {
        svg.setAttribute('width', Math.round(_naturalW * _zoomLevel));
        svg.setAttribute('height', Math.round(_naturalH * _zoomLevel));
      }
    }
    if (_zoomLabel) {
      _zoomLabel.textContent = Math.round(_zoomLevel * 100) + '%';
    }
  }

  function _zoomIn() {
    _zoomLevel = Math.min(_zoomLevel + 0.25, 5);
    _updateZoom();
  }

  function _zoomOut() {
    _zoomLevel = Math.max(_zoomLevel - 0.25, 0.25);
    _updateZoom();
  }

  function _zoomFit() {
    if (!_modalBody || _naturalW <= 0) {
      _zoomLevel = 1; _updateZoom(); return;
    }
    var bw = _modalBody.clientWidth - 48;
    if (bw > 0) {
      _zoomLevel = Math.min(bw / _naturalW, 2);
      _zoomLevel = Math.max(Math.round(_zoomLevel * 100) / 100, 0.1);
    } else {
      _zoomLevel = 1;
    }
    _updateZoom();
  }

  function ensureModal() {
    if (_modal) return;

    var overlay = document.createElement('div');
    overlay.className = 'mermaid-modal-overlay';
    overlay.addEventListener('click', closeModal);

    var modal = document.createElement('div');
    modal.className = 'mermaid-modal';
    modal.addEventListener('click', function (e) { e.stopPropagation(); });

    var header = document.createElement('div');
    header.className = 'mermaid-modal-header';
    var title = document.createElement('span');
    title.style.cssText = 'font-size:14px;font-weight:600;color:var(--text-primary,#171717);';
    title.textContent = 'Diagram';
    header.appendChild(title);

    // Zoom controls
    var zoomControls = document.createElement('div');
    zoomControls.className = 'mermaid-zoom-controls';
    var zoomOutBtn = document.createElement('button');
    zoomOutBtn.className = 'mermaid-zoom-btn';
    zoomOutBtn.textContent = '\u2212';
    zoomOutBtn.title = 'Zoom out';
    zoomOutBtn.addEventListener('click', _zoomOut);
    zoomControls.appendChild(zoomOutBtn);
    var zoomLabelEl = document.createElement('span');
    zoomLabelEl.className = 'mermaid-zoom-label';
    zoomLabelEl.textContent = '100%';
    _zoomLabel = zoomLabelEl;
    zoomControls.appendChild(zoomLabelEl);
    var zoomInBtn = document.createElement('button');
    zoomInBtn.className = 'mermaid-zoom-btn';
    zoomInBtn.textContent = '+';
    zoomInBtn.title = 'Zoom in';
    zoomInBtn.addEventListener('click', _zoomIn);
    zoomControls.appendChild(zoomInBtn);
    var zoomFitBtn = document.createElement('button');
    zoomFitBtn.className = 'mermaid-zoom-btn mermaid-zoom-fit';
    zoomFitBtn.textContent = 'Fit';
    zoomFitBtn.title = 'Fit to view';
    zoomFitBtn.addEventListener('click', _zoomFit);
    zoomControls.appendChild(zoomFitBtn);
    header.appendChild(zoomControls);

    var closeBtn = document.createElement('button');
    closeBtn.className = 'mermaid-modal-close';
    closeBtn.textContent = '\u2715';
    closeBtn.addEventListener('click', closeModal);
    header.appendChild(closeBtn);
    modal.appendChild(header);

    var body = document.createElement('div');
    body.className = 'mermaid-modal-body';
    // Mouse wheel zoom
    body.addEventListener('wheel', function (e) {
      if (!_svgWrap) return;
      e.preventDefault();
      if (e.deltaY < 0) { _zoomIn(); } else { _zoomOut(); }
    }, { passive: false });
    // Drag to pan
    var _dragging = false, _dragStartX = 0, _dragStartY = 0, _scrollStartX = 0, _scrollStartY = 0;
    body.addEventListener('mousedown', function (e) {
      if (e.button !== 0) return;
      _dragging = true;
      _dragStartX = e.clientX;
      _dragStartY = e.clientY;
      _scrollStartX = body.scrollLeft;
      _scrollStartY = body.scrollTop;
      body.style.cursor = 'grabbing';
      e.preventDefault();
    });
    document.addEventListener('mousemove', function (e) {
      if (!_dragging) return;
      body.scrollLeft = _scrollStartX - (e.clientX - _dragStartX);
      body.scrollTop = _scrollStartY - (e.clientY - _dragStartY);
    });
    document.addEventListener('mouseup', function () {
      if (_dragging) {
        _dragging = false;
        body.style.cursor = '';
      }
    });
    modal.appendChild(body);

    overlay.appendChild(modal);
    document.body.appendChild(overlay);

    _modal = overlay;
    _modalBody = body;

    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && _modal && _modal.classList.contains('open')) {
        closeModal();
      }
    });
  }

  function openMermaidModal(renderedEl) {
    ensureModal();
    _zoomLevel = 1;
    _svgWrap = null;
    _updateZoom();
    _modalBody.innerHTML = '';
    _modal.classList.add('open');

    // Clone the already-rendered SVG from the inline diagram
    var srcSvg = renderedEl.querySelector('svg');
    if (!srcSvg) {
      _modalBody.innerHTML = '<div class="mermaid-error"><span class="mermaid-error-label">No diagram found</span></div>';
      return;
    }

    var wrap = document.createElement('div');
    wrap.className = 'mermaid-svg-wrap';
    var svg = srcSvg.cloneNode(true);
    // Extract natural dimensions from viewBox
    _naturalW = 0;
    _naturalH = 0;
    var vb = svg.getAttribute('viewBox');
    if (vb) {
      var parts = vb.split(/[\s,]+/);
      _naturalW = parseFloat(parts[2]) || 0;
      _naturalH = parseFloat(parts[3]) || 0;
    }
    // Fallback to inline width/height attributes
    if (_naturalW <= 0) _naturalW = parseFloat(svg.getAttribute('width')) || 600;
    if (_naturalH <= 0) _naturalH = parseFloat(svg.getAttribute('height')) || 400;
    svg.style.maxWidth = 'none';
    svg.style.maxHeight = 'none';
    svg.style.display = 'block';
    wrap.appendChild(svg);
    _modalBody.appendChild(wrap);
    _svgWrap = wrap;
    // Auto-fit on open
    _zoomFit();
  }

  function closeModal() {
    if (_modal) {
      _modal.classList.remove('open');
      if (_modalBody) _modalBody.innerHTML = '';
      _svgWrap = null;
    }
  }

  // Re-render all mermaid diagrams when theme changes
  function reRenderAll() {
    var rendered = document.querySelectorAll('.mermaid-rendered[data-mermaid-source]');
    if (rendered.length === 0) return;

    var isDark = document.documentElement.getAttribute('data-theme') === 'dark';
    if (typeof mermaid !== 'undefined' && mermaid.initialize) {
      mermaid.initialize({ startOnLoad: false, theme: isDark ? 'dark' : 'default', securityLevel: 'loose', suppressErrorRendering: true });
    }

    for (var i = 0; i < rendered.length; i++) {
      (function(el) {
        var encoded = el.getAttribute('data-mermaid-source');
        if (!encoded) return;

        var source;
        try {
          source = decodeURIComponent(escape(atob(encoded)));
        } catch (e) { return; }

        var id = 'mermaid-rerender-' + (++_mermaidCounter);
        mermaid.render(id, source)
          .then(function(result) {
            el.innerHTML = result.svg;
          })
          .catch(function() { /* keep existing SVG on error */ });
      })(rendered[i]);
    }
  }

  // Expose via shared utils (loaded after shared.js)
  utils.renderMermaidBlocks = renderMermaidBlocks;
  utils.reRenderMermaid = reRenderAll;
})();
