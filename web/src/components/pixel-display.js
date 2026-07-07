import { LitElement, html, css } from 'lit';

const LCD_WIDTH = 160;
const LCD_HEIGHT = 144;
const SCALE = 2;
// BGB-style DMG green palette as [R, G, B]
const PALETTE = [[0xe0,0xf8,0xd0], [0x88,0xc0,0x70], [0x34,0x68,0x56], [0x08,0x18,0x20]];

/**
 * Renders console frames from trace pixel data.
 *
 * Single mode: one canvas showing the current frame.
 * Compare mode (storeB set): three canvases — A | diff | B.
 * T-cycle mode (perEntryPixels): renders partial frames at the given
 * currentIndex and supports pixel crosshair highlighting.
 *
 * Two frame sources: the GB per-entry pix replay (fixed 160×144), and
 * indexed frame snapshots (`pix_format = indexed8`) whose payloads carry
 * their own dimensions, palette, and pixel aspect.
 */
export class PixelDisplay extends LitElement {
  static properties = {
    store: { type: Object },
    storeB: { type: Object },
    nameA: { type: String },
    nameB: { type: String },
    frameBoundaries: { type: Array },
    viewStart: { type: Number },
    perEntryPixels: { type: Boolean },
    currentIndex: { type: Number },
    _frameIndex: { state: true },
    _frameCountA: { state: true },
    _highlightPixel: { state: true },
    _pixMap: { state: true },
    _pixMapFrame: { state: true },
    _dims: { state: true },
  };

  static styles = css`
    :host { display: block; }
    .pixel-wrap {
      border: 1px solid var(--border);
      border-radius: 8px;
      background: var(--bg-surface);
      padding: 8px;
    }
    .pixel-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 4px;
      font-size: 0.75rem;
    }
    .pixel-title {
      font-family: var(--mono);
      font-weight: 600;
      color: var(--accent);
    }
    .frame-info {
      color: var(--text-muted);
    }
    .canvas-wrap {
      position: relative;
      display: block;
      width: fit-content;
    }
    .highlight-overlay {
      position: absolute;
      top: 0;
      left: 0;
      pointer-events: none;
      image-rendering: pixelated;
    }
    .compare-row {
      display: flex;
      gap: 8px;
      align-items: flex-start;
    }
    .compare-panel {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 2px;
    }
    .compare-label {
      font-size: 0.7rem;
      font-family: var(--mono);
      color: var(--text-muted);
    }
    .compare-label.a { color: var(--accent); }
    .compare-label.b { color: #d29922; }
    .compare-label.diff { color: var(--red); }
    canvas {
      image-rendering: pixelated;
      border-radius: 4px;
    }
  `;

  constructor() {
    super();
    this.store = null;
    this.storeB = null;
    this.nameA = '';
    this.nameB = '';
    this.frameBoundaries = [];
    this.viewStart = 0;
    this.perEntryPixels = false;
    this.currentIndex = null;
    this._frameIndex = 0;
    this._frameCountA = 0;
    this._highlightPixel = null;
    this._pixMap = null;
    this._reversePixMap = null;
    this._pixMapFrame = -1;
    this._dims = { w: LCD_WIDTH, h: LCD_HEIGHT, aspect: 1 };
  }

  _isIndexed() {
    return this.store?.hasIndexedFrames?.() || false;
  }

  updated(changed) {
    if (changed.has('store') || changed.has('storeB')) {
      this._frameCountA = this.store?.frameCount() || 0;
      this._pixMap = null;
      this._pixMapFrame = -1;
    }
    if (changed.has('_dims')) {
      // Canvas dimension attributes changed (indexed frames size per
      // frame), which cleared the canvases — repaint them.
      this.updateComplete.then(() => this._draw());
      return;
    }
    if (changed.has('viewStart') || changed.has('frameBoundaries') ||
        changed.has('store') || changed.has('storeB') || changed.has('currentIndex')) {
      const oldFrame = this._frameIndex;
      this._syncFrameIndex();
      const frameChanged = this._frameIndex !== oldFrame;
      const storeChanged = changed.has('store') || changed.has('storeB') ||
          changed.has('viewStart') || changed.has('frameBoundaries');

      if (frameChanged || storeChanged) {
        this.updateComplete.then(() => {
          if (this.perEntryPixels && this.currentIndex != null) {
            this._updateForCurrentIndex();
          } else {
            this._draw();
          }
        });
      } else if (changed.has('currentIndex') && this.perEntryPixels && this.currentIndex != null) {
        this._updateForCurrentIndex();
      }
    }
  }

  _syncFrameIndex() {
    const bounds = this.frameBoundaries || [];
    // Use currentIndex to determine frame if available, otherwise viewStart
    const refIndex = this.currentIndex != null ? this.currentIndex : this.viewStart;
    let frame = 0;
    for (let i = 0; i < bounds.length; i++) {
      if (bounds[i] <= refIndex) frame = i;
      else break;
    }
    if (frame !== this._frameIndex) {
      this._frameIndex = frame;
      this._pixMap = null;
      this._pixMapFrame = -1;
    }
  }

  _getFrameRange() {
    const bounds = this.frameBoundaries || [];
    const fi = this._frameIndex;
    const start = bounds[fi] || 0;
    const end = fi + 1 < bounds.length ? bounds[fi + 1] : (this.store?.entryCount() || 0);
    return { start, end };
  }

  _ensurePixMap() {
    if (this._pixMapFrame === this._frameIndex || !this.store || !this.perEntryPixels) return;
    try {
      this._pixMap = this.store.buildPixelPositionMap(this._frameIndex);
      this._reversePixMap = this.store.buildReversePixelMap(this._frameIndex);
      this._pixMapFrame = this._frameIndex;
    } catch (_) {
      this._pixMap = null;
      this._reversePixMap = null;
    }
  }

  _updateForCurrentIndex() {
    if (!this.perEntryPixels || this.currentIndex == null) {
      if (this._highlightPixel) {
        this._highlightPixel = null;
        this._drawHighlight();
      }
      this._draw();
      return;
    }
    this._ensurePixMap();

    const { start } = this._getFrameRange();

    // Update pixel highlight from position map
    if (this._pixMap) {
      const mapIdx = this.currentIndex - start;
      if (mapIdx < 0 || mapIdx >= this._pixMap.length) {
        this._highlightPixel = null;
      } else {
        const packed = this._pixMap[mapIdx];
        if (packed === 0xFFFFFFFF) {
          this._highlightPixel = null;
        } else {
          this._highlightPixel = { x: packed >> 16, y: packed & 0xFFFF };
        }
      }
      this._drawHighlight();
    }

    // Draw partial frame at current index
    if (this.currentIndex >= start) {
      this._drawPartialAt(this.currentIndex);
      // In compare mode, also update B and diff
      if (this.storeB) {
        const fi = this._frameIndex;
        this._renderToCanvas('canvasB', this.storeB, fi);
        const pixA = this.store?.renderFrame(fi);
        const pixB = this.storeB?.renderFrame(fi);
        if (pixA && pixB) this._renderDiff(pixA, pixB);
      }
    }
  }

  /** Draw a partial frame at a specific entry index. */
  _drawPartialAt(entry) {
    if (!this.store || !this.perEntryPixels) return;
    const canvas = this.renderRoot?.querySelector('#canvasA');
    if (!canvas) return;
    try {
      const ctx = canvas.getContext('2d');
      if (!this._tmpCanvas) {
        this._tmpCanvas = document.createElement('canvas');
        this._tmpCanvas.width = LCD_WIDTH;
        this._tmpCanvas.height = LCD_HEIGHT;
      }
      const tmp = this._tmpCanvas.getContext('2d');

      // Draw the full completed frame as a faded background
      const fullRgba = this.store.renderFrame(this._frameIndex);
      if (fullRgba) {
        const fullArr = new Uint8ClampedArray(fullRgba.buffer || fullRgba);
        tmp.putImageData(new ImageData(fullArr, LCD_WIDTH, LCD_HEIGHT), 0, 0);
        this._drawCheckerboard(ctx);
        ctx.globalAlpha = 0.25;
        ctx.drawImage(this._tmpCanvas, 0, 0);
        ctx.globalAlpha = 1.0;
      } else {
        this._drawCheckerboard(ctx);
      }

      // Draw the partial frame (up to current entry) at full opacity on top
      const rgba = this.store.renderPartialFrame(this._frameIndex, entry);
      if (!rgba) return;
      const arr = new Uint8ClampedArray(rgba.buffer || rgba);
      tmp.putImageData(new ImageData(arr, LCD_WIDTH, LCD_HEIGHT), 0, 0);
      ctx.drawImage(this._tmpCanvas, 0, 0);
    } catch (err) {
      console.error('Failed to render partial frame:', err);
    }
  }

  _drawHighlight() {
    const overlay = this.renderRoot?.querySelector('.highlight-overlay');
    if (!overlay) return;
    const ctx = overlay.getContext('2d');
    ctx.clearRect(0, 0, LCD_WIDTH, LCD_HEIGHT);
    if (!this._highlightPixel) return;
    const { x, y } = this._highlightPixel;
    ctx.strokeStyle = '#ff4444';
    ctx.lineWidth = 1;
    // Draw crosshair
    ctx.beginPath();
    ctx.moveTo(x, 0); ctx.lineTo(x, LCD_HEIGHT);
    ctx.moveTo(0, y + 0.5); ctx.lineTo(LCD_WIDTH, y + 0.5);
    ctx.stroke();
    // Draw pixel highlight
    ctx.fillStyle = 'rgba(255,68,68,0.5)';
    ctx.fillRect(x, y, 1, 1);
  }

  _drawCheckerboard(ctx) {
    const size = 4; // checker size in LCD pixels
    for (let y = 0; y < LCD_HEIGHT; y += size) {
      for (let x = 0; x < LCD_WIDTH; x += size) {
        const dark = ((x / size) + (y / size)) % 2 === 0;
        ctx.fillStyle = dark ? '#1a1a2e' : '#16213e';
        ctx.fillRect(x, y, size, size);
      }
    }
  }

  _renderToCanvas(id, store, frameIndex) {
    const canvas = this.renderRoot?.querySelector(`#${id}`);
    if (!canvas) return null;
    const ctx = canvas.getContext('2d');
    if (!store) { ctx.clearRect(0, 0, LCD_WIDTH, LCD_HEIGHT); return null; }
    try {
      const rgba = store.renderFrame(frameIndex);
      if (!rgba) { ctx.clearRect(0, 0, LCD_WIDTH, LCD_HEIGHT); return null; }
      const arr = new Uint8ClampedArray(rgba.buffer || rgba);
      // Draw checkerboard for unrendered areas, then composite frame over it
      this._drawCheckerboard(ctx);
      if (!this._tmpCanvas) {
        this._tmpCanvas = document.createElement('canvas');
        this._tmpCanvas.width = LCD_WIDTH;
        this._tmpCanvas.height = LCD_HEIGHT;
      }
      const tmp = this._tmpCanvas.getContext('2d');
      tmp.putImageData(new ImageData(arr, LCD_WIDTH, LCD_HEIGHT), 0, 0);
      ctx.drawImage(this._tmpCanvas, 0, 0);
      return arr;
    } catch (err) {
      console.error('Failed to render frame:', err);
      ctx.clearRect(0, 0, LCD_WIDTH, LCD_HEIGHT);
      return null;
    }
  }

  _renderDiff(rgbaA, rgbaB, w = LCD_WIDTH, h = LCD_HEIGHT) {
    const canvas = this.renderRoot?.querySelector('#diff');
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!rgbaA || !rgbaB) { ctx.clearRect(0, 0, canvas.width, canvas.height); return; }
    // Compare rendered RGBA pixels — format-agnostic, so it works for both
    // DMG (greyscale) and CGB (colour) frames.
    const a = new Uint8ClampedArray(rgbaA.buffer || rgbaA);
    const b = new Uint8ClampedArray(rgbaB.buffer || rgbaB);
    const diff = new Uint8ClampedArray(w * h * 4);
    for (let i = 0; i < w * h; i++) {
      const off = i * 4;
      const same = a[off] === b[off] && a[off+1] === b[off+1]
        && a[off+2] === b[off+2] && a[off+3] === b[off+3];
      if (same) {
        // Identical — show A's pixel as-is
        diff[off] = a[off]; diff[off+1] = a[off+1];
        diff[off+2] = a[off+2]; diff[off+3] = a[off+3];
      } else {
        // Different — highlight in red, modulated by A's luminance
        const brightness = 1 - (a[off] + a[off+1] + a[off+2]) / (3 * 255);
        diff[off] = Math.round(180 + 75 * brightness);
        diff[off+1] = Math.round(30 + 40 * brightness);
        diff[off+2] = Math.round(30 + 40 * brightness);
        diff[off+3] = 255;
      }
    }
    ctx.putImageData(new ImageData(diff, w, h), 0, 0);
  }

  _canvasToLcd(e) {
    // Find the nearest canvas element from the event target
    const canvas = e.target.closest('canvas') || this.renderRoot?.querySelector('#canvasA');
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor((e.clientX - rect.left) / SCALE);
    const y = Math.floor((e.clientY - rect.top) / SCALE);
    if (x < 0 || x >= LCD_WIDTH || y < 0 || y >= LCD_HEIGHT) return null;
    return { x, y };
  }

  _entryAtPixel(x, y) {
    this._ensurePixMap();
    if (!this._reversePixMap) return null;
    const idx = y * LCD_WIDTH + x;
    const entry = this._reversePixMap[idx];
    return (entry === 0xFFFFFFFF) ? null : entry;
  }

  _onCanvasMouseMove(e) {
    const pos = this._canvasToLcd(e);
    if (!pos) return;
    const entry = this._entryAtPixel(pos.x, pos.y);
    if (entry != null) {
      this.dispatchEvent(new CustomEvent('hover-index', {
        detail: { index: entry }, bubbles: true, composed: true,
      }));
    }
  }

  _onCanvasMouseLeave() {
    this.dispatchEvent(new CustomEvent('hover-index', {
      detail: { index: null }, bubbles: true, composed: true,
    }));
  }

  _onCanvasClick(e) {
    const pos = this._canvasToLcd(e);
    if (!pos) return;
    const entry = this._entryAtPixel(pos.x, pos.y);
    if (entry != null) {
      this.dispatchEvent(new CustomEvent('current-index', {
        detail: { index: entry }, bubbles: true, composed: true,
      }));
    }
  }

  /** Draw an indexed frame snapshot; returns {width, height, rgba} or null. */
  _renderIndexedToCanvas(id, store, frameIndex) {
    const canvas = this.renderRoot?.querySelector(`#${id}`);
    if (!canvas) return null;
    const ctx = canvas.getContext('2d');
    if (!store) { ctx.clearRect(0, 0, canvas.width, canvas.height); return null; }
    try {
      const frame = store.indexedFrame(frameIndex);
      if (!frame) { ctx.clearRect(0, 0, canvas.width, canvas.height); return null; }
      if (canvas.width !== frame.width || canvas.height !== frame.height) {
        canvas.width = frame.width;
        canvas.height = frame.height;
      }
      const arr = new Uint8ClampedArray(frame.rgba.buffer || frame.rgba);
      ctx.putImageData(new ImageData(arr, frame.width, frame.height), 0, 0);
      return { width: frame.width, height: frame.height, aspect: frame.pixelAspect || 1, rgba: arr };
    } catch (err) {
      console.error('Failed to render indexed frame:', err);
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      return null;
    }
  }

  _drawIndexed() {
    const fi = this._frameIndex;
    const a = this._renderIndexedToCanvas('canvasA', this.store, fi);
    if (a) {
      const d = this._dims;
      if (d.w !== a.width || d.h !== a.height || d.aspect !== a.aspect) {
        this._dims = { w: a.width, h: a.height, aspect: a.aspect };
      }
    }
    if (this.storeB) {
      const b = this._renderIndexedToCanvas('canvasB', this.storeB, fi);
      if (a && b && a.width === b.width && a.height === b.height) {
        this._renderDiff(a.rgba, b.rgba, a.width, a.height);
      } else {
        this._renderDiff(null, null);
      }
    }
  }

  _draw() {
    if (this._isIndexed()) {
      this._drawIndexed();
      return;
    }
    const fi = this._frameIndex;
    if (this.storeB) {
      if (this.perEntryPixels && this.currentIndex != null) {
        this._drawPartialAt(this.currentIndex);
      } else {
        this._renderToCanvas('canvasA', this.store, fi);
      }
      this._renderToCanvas('canvasB', this.storeB, fi);
      // Diff compares rendered RGBA from A vs B (works for DMG and CGB colour)
      const pixA = this.store?.renderFrame(fi);
      const pixB = this.storeB?.renderFrame(fi);
      if (pixA && pixB) this._renderDiff(pixA, pixB);
    } else if (this.currentIndex != null && this.perEntryPixels) {
      this._drawPartialAt(this.currentIndex);
    } else {
      this._renderToCanvas('canvasA', this.store, fi);
    }
  }

  render() {
    const total = this._frameCountA;
    const { w: W, h: H, aspect } = this._dims;
    const cssW = Math.round(W * SCALE * aspect);
    const cssH = H * SCALE;

    if (this.storeB) {
      return html`
        <div class="pixel-wrap">
          <div class="pixel-header">
            <span class="pixel-title">pixels</span>
            <span class="frame-info">frame ${this._frameIndex + 1} / ${total}</span>
          </div>
          <div class="compare-row"
            @mousemove=${this.perEntryPixels ? this._onCanvasMouseMove : null}
            @mouseleave=${this.perEntryPixels ? this._onCanvasMouseLeave : null}
            @click=${this.perEntryPixels ? this._onCanvasClick : null}
            style="${this.perEntryPixels ? 'cursor:crosshair;' : ''}">
            <div class="compare-panel">
              <span class="compare-label a">${this.nameA || 'A'}</span>
              <div class="canvas-wrap">
                <canvas id="canvasA" width=${W} height=${H}
                  style="width: ${cssW}px; height: ${cssH}px;"></canvas>
                ${this.perEntryPixels ? html`
                  <canvas class="highlight-overlay" width=${W} height=${H}
                    style="width: ${cssW}px; height: ${cssH}px;"></canvas>
                ` : ''}
              </div>
            </div>
            <div class="compare-panel">
              <span class="compare-label diff">diff</span>
              <canvas id="diff" width=${W} height=${H}
                style="width: ${cssW}px; height: ${cssH}px;"></canvas>
            </div>
            <div class="compare-panel">
              <span class="compare-label b">${this.nameB || 'B'}</span>
              <canvas id="canvasB" width=${W} height=${H}
                style="width: ${cssW}px; height: ${cssH}px;"></canvas>
            </div>
          </div>
        </div>
      `;
    }

    return html`
      <div class="pixel-wrap">
        <div class="pixel-header">
          <span class="pixel-title">pixels</span>
          <span class="frame-info">frame ${this._frameIndex + 1} / ${total}</span>
        </div>
        <div class="canvas-wrap"
          @mousemove=${this.perEntryPixels ? this._onCanvasMouseMove : null}
          @mouseleave=${this.perEntryPixels ? this._onCanvasMouseLeave : null}
          @click=${this.perEntryPixels ? this._onCanvasClick : null}
          style="${this.perEntryPixels ? 'cursor:crosshair;' : ''}">
          <canvas id="canvasA" width=${W} height=${H}
            style="width: ${cssW}px; height: ${cssH}px;"></canvas>
          ${this.perEntryPixels ? html`
            <canvas class="highlight-overlay" width=${W} height=${H}
              style="width: ${cssW}px; height: ${cssH}px;"></canvas>
          ` : ''}
        </div>
      </div>
    `;
  }
}

customElements.define('pixel-display', PixelDisplay);
