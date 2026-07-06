import { useState, useEffect, useCallback, useRef } from 'react';

interface Props {
  src: string;
  onClose: () => void;
}

interface PixelInfo {
  x: number;
  y: number;
  r: number;
  g: number;
  b: number;
  a: number;
  screenX: number;
  screenY: number;
}

export function ImageLightbox({ src, onClose }: Props) {
  const [scale, setScale] = useState(1);
  const [tx, setTx] = useState(0);
  const [ty, setTy] = useState(0);
  const dragging = useRef(false);
  const lastPos = useRef({ x: 0, y: 0 });
  const imgRef = useRef<HTMLImageElement>(null);
  const imageDataRef = useRef<ImageData | null>(null);
  const naturalSize = useRef({ w: 0, h: 0 });

  const [pickerActive, setPickerActive] = useState(false);
  const [pixelInfo, setPixelInfo] = useState<PixelInfo | null>(null);

  useEffect(() => {
    const img = new Image();
    img.onload = () => {
      naturalSize.current = { w: img.naturalWidth, h: img.naturalHeight };
      const canvas = document.createElement('canvas');
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(img, 0, 0);
      imageDataRef.current = ctx.getImageData(0, 0, img.naturalWidth, img.naturalHeight);
    };
    img.src = src;
  }, [src]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (pickerActive) {
          setPickerActive(false);
          setPixelInfo(null);
        } else {
          onClose();
        }
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, pickerActive]);

  const reset = useCallback(() => {
    setScale(1);
    setTx(0);
    setTy(0);
  }, []);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    setScale((s) => Math.min(20, Math.max(0.1, s * factor)));
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0 || pickerActive) return;
    dragging.current = true;
    lastPos.current = { x: e.clientX, y: e.clientY };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [pickerActive]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (pickerActive && imgRef.current && imageDataRef.current) {
      const rect = imgRef.current.getBoundingClientRect();
      const { w, h } = naturalSize.current;
      const px = Math.floor((e.clientX - rect.left) / rect.width * w);
      const py = Math.floor((e.clientY - rect.top) / rect.height * h);
      if (px >= 0 && px < w && py >= 0 && py < h) {
        const i = (py * w + px) * 4;
        const d = imageDataRef.current.data;
        setPixelInfo({
          x: px, y: py,
          r: d[i], g: d[i + 1], b: d[i + 2], a: d[i + 3],
          screenX: e.clientX, screenY: e.clientY,
        });
      } else {
        setPixelInfo(null);
      }
      return;
    }
    if (!dragging.current) return;
    const dx = e.clientX - lastPos.current.x;
    const dy = e.clientY - lastPos.current.y;
    lastPos.current = { x: e.clientX, y: e.clientY };
    setTx((v) => v + dx);
    setTy((v) => v + dy);
  }, [pickerActive]);

  const handlePointerUp = useCallback(() => {
    dragging.current = false;
  }, []);

  const handleBackdropClick = useCallback((e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  }, [onClose]);

  const handleSave = useCallback(async () => {
    const byteString = atob(src.split(',')[1]);
    const mime = src.split(':')[1].split(';')[0];
    const ab = new ArrayBuffer(byteString.length);
    const ia = new Uint8Array(ab);
    for (let i = 0; i < byteString.length; i++) ia[i] = byteString.charCodeAt(i);
    const blob = new Blob([ab], { type: mime });

    if (typeof showSaveFilePicker === 'function') {
      try {
        const handle = await showSaveFilePicker({
          suggestedName: 'preview.png',
          types: [{ description: 'PNG Image', accept: { 'image/png': ['.png'] } }],
        });
        const writable = await handle.createWritable();
        await writable.write(blob);
        await writable.close();
        return;
      } catch {
        return;
      }
    }

    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'preview.png';
    document.body.appendChild(a);
    a.click();
    requestAnimationFrame(() => {
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    });
  }, [src]);

  const imgCursor = pickerActive ? 'crosshair' : dragging.current ? 'grabbing' : 'grab';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80"
      onClick={handleBackdropClick}
      onWheel={handleWheel}
    >
      {/* Toolbar */}
      <div
        className="absolute top-3 left-1/2 -translate-x-1/2 z-20 flex items-center gap-1 px-2 py-1.5 rounded-lg bg-black/50 backdrop-blur-sm"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Save as PNG */}
        <button
          onClick={handleSave}
          className="w-8 h-8 flex items-center justify-center rounded hover:bg-white/15 transition-colors"
          title="Save as PNG"
        >
          <svg viewBox="0 0 16 16" className="w-4 h-4" fill="none" stroke="white" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M2 1a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V4.414a1 1 0 0 0-.293-.707l-2.414-2.414A1 1 0 0 0 11.586 1H2z" />
            <path d="M3 1v3a1 1 0 0 0 1 1h4a1 1 0 0 0 1-1V1" />
            <path d="M5 9a2 2 0 1 1 4 0 2 2 0 0 1-4 0z" />
          </svg>
        </button>

        {/* Color Picker toggle */}
        <button
          onClick={() => { setPickerActive((v) => !v); setPixelInfo(null); }}
          className={`w-8 h-8 flex items-center justify-center rounded transition-colors ${pickerActive ? 'bg-white/25 text-white' : 'text-white/70 hover:bg-white/15 hover:text-white'}`}
          title="Color Picker"
        >
          <svg viewBox="0 0 16 16" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
            <circle cx="8" cy="8" r="6" />
            <line x1="8" y1="0.5" x2="8" y2="4" />
            <line x1="8" y1="12" x2="8" y2="15.5" />
            <line x1="0.5" y1="8" x2="4" y2="8" />
            <line x1="12" y1="8" x2="15.5" y2="8" />
          </svg>
        </button>

        <div className="w-px h-4 bg-white/20 mx-0.5" />

        {/* Close button */}
        <button
          onClick={onClose}
          className="w-8 h-8 flex items-center justify-center text-white/70 hover:text-white hover:bg-white/15 rounded transition-colors text-lg leading-none"
        >
          ✕
        </button>
      </div>

      <img
        ref={imgRef}
        src={src}
        alt="preview"
        draggable={false}
        onDoubleClick={reset}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        className="max-w-[90vw] max-h-[90vh] object-contain select-none"
        style={{
          transform: `translate(${tx}px, ${ty}px) scale(${scale})`,
          cursor: imgCursor,
          imageRendering: 'pixelated',
        }}
      />

      {/* Picker tooltip */}
      {pickerActive && pixelInfo && (
        <div
          className="fixed z-30 pointer-events-none flex items-center gap-1.5 px-2 py-1 rounded bg-black/75 backdrop-blur-sm text-[11px] text-white font-mono whitespace-nowrap"
          style={{ left: pixelInfo.screenX + 16, top: pixelInfo.screenY + 16 }}
        >
          <span
            className="w-3 h-3 rounded-sm border border-white/30 shrink-0"
            style={{ background: `rgb(${pixelInfo.r},${pixelInfo.g},${pixelInfo.b})` }}
          />
          <span>({pixelInfo.x}, {pixelInfo.y})</span>
          <span className="text-white/50">|</span>
          <span>R{pixelInfo.r} G{pixelInfo.g} B{pixelInfo.b} A{pixelInfo.a}</span>
        </div>
      )}
    </div>
  );
}
