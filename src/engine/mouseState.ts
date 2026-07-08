export class MouseState {
  readonly iMouse: Float32Array = new Float32Array(4);
  private el: HTMLElement | null = null;

  // Bound handlers created once — stable references for addEventListener/removeEventListener
  private readonly onMove: (e: MouseEvent) => void;
  private readonly onDown: (e: MouseEvent) => void;
  private readonly onUp: (e: MouseEvent) => void;

  constructor() {
    this.onMove = (e: MouseEvent) => {
      if (!this.el) return;
      this.iMouse[0] = e.offsetX;
      this.iMouse[1] = this.el.clientHeight - e.offsetY; // bottom-left origin
    };

    this.onDown = (e: MouseEvent) => {
      if (!this.el) return;
      const x = e.offsetX;
      const y = this.el.clientHeight - e.offsetY;
      this.iMouse[2] = Math.abs(x) || 1; // positive = pressed (avoid 0 which reads as "no click")
      this.iMouse[3] = Math.abs(y) || 1;
    };

    this.onUp = () => {
      // Shadertoy convention: negate z/w on release
      this.iMouse[2] = -Math.abs(this.iMouse[2]);
      this.iMouse[3] = -Math.abs(this.iMouse[3]);
    };
  }

  attach(el: HTMLElement): void {
    // Detach from previous element if any
    this.detach();
    this.el = el;
    el.addEventListener('mousemove', this.onMove);
    el.addEventListener('mousedown', this.onDown);
    el.addEventListener('mouseup', this.onUp);
  }

  detach(): void {
    if (!this.el) return;
    this.el.removeEventListener('mousemove', this.onMove);
    this.el.removeEventListener('mousedown', this.onDown);
    this.el.removeEventListener('mouseup', this.onUp);
    this.el = null;
  }
}
