import { mkdir, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const handleKey = (handle) => `${handle.generation ?? '0'}:${handle.index ?? '0'}`;

/** A transport, JSON-RPC, or MCP tool failure. The original response is retained. */
export class SlintMcpError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = 'SlintMcpError';
    Object.assign(this, details);
  }
}

/** Decode MCP text content without discarding non-JSON text or image content. */
export function toolValue(result) {
  if (result.structuredContent !== undefined) return result.structuredContent;
  const texts = (result.content ?? []).filter((part) => part.type === 'text').map((part) => {
    try { return JSON.parse(part.text); } catch { return part.text; }
  });
  return texts.length === 1 ? texts[0] : texts.length ? texts : result;
}

/** Dependency-free HTTP MCP client for Slint's built-in automation server. */
export class SlintUiClient {
  constructor(endpoint, { timeoutMs = 15_000, windowId, headers = {}, signal } = {}) {
    const url = new URL(endpoint);
    if (!['http:', 'https:'].includes(url.protocol)) throw new TypeError('MCP endpoint must be HTTP(S)');
    this.endpoint = url.href;
    this.timeoutMs = timeoutMs;
    this.windowId = windowId;
    this.headers = headers;
    this.signal = signal;
    this.nextId = 1;
  }

  async request(method, params = {}, { notification = false } = {}) {
    this.signal?.throwIfAborted();
    const id = notification ? undefined : this.nextId++;
    const response = await fetch(this.endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json', Accept: 'application/json, text/event-stream',
        ...(this.sessionId ? { 'Mcp-Session-Id': this.sessionId } : {}),
        ...(this.protocolVersion ? { 'MCP-Protocol-Version': this.protocolVersion } : {}),
        ...this.headers,
      },
      body: JSON.stringify({ jsonrpc: '2.0', ...(notification ? {} : { id }), method, params }),
      signal: this.signal ? AbortSignal.any([this.signal, AbortSignal.timeout(this.timeoutMs)]) : AbortSignal.timeout(this.timeoutMs),
    });
    this.sessionId = response.headers.get('mcp-session-id') ?? this.sessionId;
    const body = await response.text();
    if (!response.ok) throw new SlintMcpError(`MCP HTTP ${response.status}: ${body}`, { method, status: response.status, body });
    if (notification && !body.trim()) return undefined;
    let envelope;
    try {
      if (response.headers.get('content-type')?.includes('text/event-stream')) {
        for (const event of body.split(/\r?\n\r?\n/)) {
          const data = event.split(/\r?\n/).filter((line) => line.startsWith('data:')).map((line) => line.slice(5).trimStart()).join('\n');
          if (!data) continue;
          const candidate = JSON.parse(data);
          if (candidate.id === id) envelope = candidate;
        }
        if (!envelope) throw new Error('No matching response in MCP event stream');
      } else {
        envelope = JSON.parse(body);
      }
    } catch (cause) {
      throw new SlintMcpError(`Invalid MCP response for ${method}: ${cause.message}`, { method, body, cause });
    }
    if (envelope.error) throw new SlintMcpError(`MCP ${method}: ${envelope.error.message}`, { method, code: envelope.error.code, data: envelope.error.data, response: envelope });
    if (notification) return envelope.result;
    if (envelope.id !== id || !Object.hasOwn(envelope, 'result')) throw new SlintMcpError(`Mismatched MCP response for ${method}`, { method, response: envelope });
    return envelope.result;
  }

  async init() {
    this.server = await this.request('initialize', {
      protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'open-quartz-slint-ui', version: '1.0.0' },
    });
    this.protocolVersion = this.server.protocolVersion;
    await this.request('notifications/initialized', {}, { notification: true });
    return this.server;
  }

  async call(name, args = {}) {
    const result = await this.request('tools/call', { name, arguments: args });
    if (result.isError) throw new SlintMcpError(`MCP tool ${name}: ${JSON.stringify(toolValue(result))}`, { tool: name, arguments: args, response: result });
    return result;
  }

  async value(name, args = {}) { return toolValue(await this.call(name, args)); }
  async tools() { return this.request('tools/list'); }

  async windows() { return (await this.value('list_windows')).windowHandles ?? []; }

  async useWindow(handle) {
    this.windowId = handle ?? (await this.windows())[0];
    if (!this.windowId) throw new SlintMcpError('No Slint window is available');
    const properties = await this.windowProperties();
    if (!properties.rootElementHandle) throw new SlintMcpError('Window has no rootElementHandle');
    this.rootHandle = properties.rootElementHandle;
    return properties;
  }

  async windowProperties(handle = this.windowId) {
    return this.value('get_window_properties', { windowHandle: handle });
  }

  handle(element) {
    const handle = element?.handle ?? element;
    if (!handle || typeof handle !== 'object') throw new TypeError('Expected an element handle or tree element');
    return handle;
  }

  async properties(element) {
    return this.value('get_element_properties', { elementHandle: this.handle(element) });
  }

  async tree({ root = this.rootHandle, maxElements = 1000, path } = {}) {
    if (!root) { await this.useWindow(); root = this.rootHandle; }
    const tree = await this.value('get_element_tree', { elementHandle: this.handle(root), maxElements });
    if (path) await saveJson(path, tree);
    return tree;
  }

  /** Labels are matched locally because Slint has no label query instruction. */
  async query({ id, type, label, role, root, visible = true, enabled } = {}) {
    if (!this.rootHandle) await this.useWindow();
    let elements;
    if (id || type || role) {
      let handles;
      if (id && !type && !role && !root) {
        handles = (await this.value('find_elements_by_id', { windowHandle: this.windowId, elementsId: id })).elementHandles ?? [];
      } else {
        const queryStack = [{ matchDescendants: true }];
        if (id) queryStack.push({ matchElementId: id });
        if (type) queryStack.push({ matchElementTypeNameOrBase: type });
        if (role) queryStack.push({ matchElementAccessibleRole: role });
        handles = (await this.value('query_element_descendants', {
          elementHandle: this.handle(root ?? this.rootHandle), queryStack, findAll: true,
        })).elementHandles ?? [];
      }
      handles = [...new Map(handles.map((handle) => [handleKey(handle), handle])).values()];
      elements = await Promise.all(handles.map(async (handle) => ({ ...await this.properties(handle), handle })));
    } else {
      const tree = await this.tree({ root: root ?? this.rootHandle });
      if (tree.truncated) throw new SlintMcpError('UI tree truncated; use an ID/type/role or narrower root query', { tree });
      elements = tree.elements ?? [];
    }
    elements = [...new Map(elements.map((element) => [handleKey(element.handle), element])).values()];
    return elements.filter((element) => {
      if (label instanceof RegExp) { label.lastIndex = 0; if (!label.test(element.accessibleLabel ?? '')) return false; }
      else if (label !== undefined && element.accessibleLabel !== label) return false;
      if (enabled !== undefined && Boolean(element.accessibleEnabled) !== enabled) return false;
      return !visible || (element.size?.width > 0 && element.size?.height > 0 && (element.computedOpacity ?? 0) > 0);
    });
  }

  async find(selector) {
    const elements = await this.query(typeof selector === 'string' ? { label: selector } : selector);
    if (elements.length !== 1) throw new SlintMcpError(`Expected one element for ${JSON.stringify(selector)}, found ${elements.length}`, { matches: elements });
    return elements[0];
  }

  async byId(id, options = {}) { return this.query({ ...options, id }); }
  async byType(type, options = {}) { return this.query({ ...options, type }); }
  async byLabel(label, options = {}) { return this.query({ ...options, label }); }

  async click(element, { button = 'Left', action = 'SingleClick', modifiers } = {}) {
    return this.value('click_element', { elementHandle: this.handle(element), button, action, ...(modifiers ? { modifiers } : {}) });
  }

  async activate(element, action = 'Default_') {
    return this.value('invoke_accessibility_action', { elementHandle: this.handle(element), action });
  }

  /** target is a window-local logical position, not a delta or physical screen pixel. */
  async drag(element, target, { button = 'Left', modifiers } = {}) {
    if (!Number.isFinite(target?.x) || !Number.isFinite(target?.y)) throw new TypeError('Drag target needs finite x/y');
    return this.value('drag_element', { elementHandle: this.handle(element), target, button, ...(modifiers ? { modifiers } : {}) });
  }

  async key(text, eventType = 'PressAndRelease') {
    return this.value('dispatch_key_event', { windowHandle: this.windowId, text, eventType });
  }

  async setValue(element, value) {
    return this.value('set_element_value', { elementHandle: this.handle(element), value: String(value) });
  }

  async screenshot(path) {
    const result = await this.call('take_screenshot', { windowHandle: this.windowId });
    const image = result.content?.find((part) => part.type === 'image' && part.mimeType === 'image/png');
    if (!image?.data) throw new SlintMcpError('take_screenshot returned no PNG image', { response: result });
    const png = Buffer.from(image.data, 'base64');
    if (png.length < 8 || !png.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) throw new SlintMcpError('take_screenshot returned an invalid PNG');
    if (path) { await mkdir(dirname(path), { recursive: true }); await writeFile(path, png); }
    return png;
  }

  /** Poll an observable UI condition; failed actions are never silently retried. */
  async waitFor(probe, { timeoutMs = this.timeoutMs, intervalMs = 100, description = 'UI condition' } = {}) {
    const deadline = Date.now() + timeoutMs;
    let last;
    do {
      this.signal?.throwIfAborted();
      last = await probe();
      if (last) return last;
      await delay(intervalMs, undefined, { signal: this.signal });
    } while (Date.now() < deadline);
    throw new SlintMcpError(`Timed out waiting for ${description}`, { last });
  }
}

export async function createClient(endpoint, options) {
  const client = new SlintUiClient(endpoint, options);
  await client.init();
  return client;
}

export async function saveJson(path, value) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
  return path;
}
