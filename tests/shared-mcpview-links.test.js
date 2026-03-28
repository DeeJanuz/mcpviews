import './shared-mcpview-setup.js';
import { describe, it, expect } from 'vitest';

var renderer = globalThis.__testRenderer;

describe('mcpview:// link rendering', function () {
  it('renders mcpview:// links as invocation buttons', function () {
    var result = renderer.link({
      href: 'mcpview://decision_detail?id=dec-123',
      title: null,
      text: 'View Decision'
    });
    expect(result).toContain('data-invoke-renderer="decision_detail"');
    expect(result).toContain('View Decision');
    expect(result).toContain('mcpview-invoke-btn');
  });

  it('parses multiple query params', function () {
    var result = renderer.link({
      href: 'mcpview://task_detail?id=task-456&project=proj-789',
      title: null,
      text: 'Task'
    });
    expect(result).toContain('data-invoke-renderer="task_detail"');
    var paramsMatch = result.match(/data-invoke-params="([^"]+)"/);
    expect(paramsMatch).not.toBeNull();
    var params = JSON.parse(paramsMatch[1].replace(/&amp;/g, '&').replace(/&quot;/g, '"'));
    expect(params.id).toBe('task-456');
    expect(params.project).toBe('proj-789');
  });

  it('handles mcpview:// with no query params', function () {
    var result = renderer.link({
      href: 'mcpview://simple_renderer',
      title: null,
      text: 'Simple'
    });
    expect(result).toContain('data-invoke-renderer="simple_renderer"');
    expect(result).toContain('Simple');
  });

  it('uses custom title when provided', function () {
    var result = renderer.link({
      href: 'mcpview://test?id=1',
      title: 'Custom Title',
      text: 'Link'
    });
    expect(result).toContain('title="Custom Title"');
  });

  it('generates default title from renderer name', function () {
    var result = renderer.link({
      href: 'mcpview://decision_detail?id=1',
      title: null,
      text: 'Link'
    });
    expect(result).toContain('title="Open decision detail"');
  });

  it('still renders cite: links correctly', function () {
    var result = renderer.link({
      href: 'cite:doc:1',
      title: null,
      text: '1'
    });
    expect(result).toContain('cite-link');
    expect(result).toContain('data-cite-type="doc"');
  });

  it('still renders regular links as anchor tags', function () {
    var result = renderer.link({
      href: 'https://example.com',
      title: null,
      text: 'Example'
    });
    expect(result).toContain('<a href=');
    expect(result).toContain('Example');
  });

  it('decodes URI-encoded params', function () {
    var result = renderer.link({
      href: 'mcpview://test?name=hello%20world&key=a%26b',
      title: null,
      text: 'Encoded'
    });
    var paramsMatch = result.match(/data-invoke-params="([^"]+)"/);
    var decoded = paramsMatch[1].replace(/&amp;/g, '&').replace(/&quot;/g, '"');
    var params = JSON.parse(decoded);
    expect(params.name).toBe('hello world');
    expect(params.key).toBe('a&b');
  });
});
