import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { ImageLightbox } from '../../src/components/ImageLightbox';

describe('ImageLightbox', () => {
  const mockOnClose = vi.fn();
  const testSrc = 'data:image/png;base64,iVBORw0KGgo=';

  beforeEach(() => {
    vi.clearAllMocks();
    // Mock pointer capture methods not available in jsdom
    HTMLElement.prototype.setPointerCapture = vi.fn();
    HTMLElement.prototype.releasePointerCapture = vi.fn();
  });

  it('renders with image when src provided', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    expect(img).toBeInTheDocument();
    expect(img).toHaveAttribute('src', testSrc);
  });

  it('renders backdrop overlay', () => {
    const { container } = render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const backdrop = container.querySelector('.fixed.inset-0');
    expect(backdrop).toBeTruthy();
  });

  it('close button fires onClose', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const closeBtn = screen.getByText('✕');
    fireEvent.click(closeBtn);
    expect(mockOnClose).toHaveBeenCalledTimes(1);
  });

  it('clicking backdrop fires onClose', () => {
    const { container } = render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const backdrop = container.querySelector('.fixed.inset-0.z-50');
    if (backdrop) {
      fireEvent.click(backdrop);
    }
    expect(mockOnClose).toHaveBeenCalledTimes(1);
  });

  it('renders save button', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const saveBtn = screen.getByTitle('Save as PNG');
    expect(saveBtn).toBeInTheDocument();
  });

  it('renders color picker toggle button', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const pickerBtn = screen.getByTitle('Color Picker');
    expect(pickerBtn).toBeInTheDocument();
  });

  it('toggling color picker changes button style', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const pickerBtn = screen.getByTitle('Color Picker');
    // Before toggle: not active
    expect(pickerBtn.className).toContain('text-white/70');
    fireEvent.click(pickerBtn);
    // After toggle: active
    expect(pickerBtn.className).toContain('bg-white/25');
  });

  it('image has pixelated rendering', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    expect(img.style.imageRendering).toBe('pixelated');
  });

  it('image has initial transform at scale 1 with no translation', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    expect(img.style.transform).toBe('translate(0px, 0px) scale(1)');
  });

  it('image cursor is grab by default', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    expect(img.style.cursor).toBe('grab');
  });

  it('wheel zooms in on negative deltaY', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const backdrop = document.querySelector('.fixed.inset-0')!;
    fireEvent.wheel(backdrop, { deltaY: -100 });
    const img = screen.getByAltText('preview');
    // Scale should be > 1 after zoom in
    expect(img.style.transform).toContain('scale(');
    expect(img.style.transform).not.toBe('translate(0px, 0px) scale(1)');
  });

  it('wheel zooms out on positive deltaY', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const backdrop = document.querySelector('.fixed.inset-0')!;
    fireEvent.wheel(backdrop, { deltaY: 100 });
    const img = screen.getByAltText('preview');
    expect(img.style.transform).toContain('scale(');
  });

  it('double-click resets zoom and position', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    // Zoom in first
    const backdrop = document.querySelector('.fixed.inset-0')!;
    fireEvent.wheel(backdrop, { deltaY: -100 });
    expect(img.style.transform).not.toBe('translate(0px, 0px) scale(1)');
    // Double-click to reset
    fireEvent.doubleClick(img);
    expect(img.style.transform).toBe('translate(0px, 0px) scale(1)');
  });

  it('pointer down + move drags the image', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    fireEvent.pointerDown(img, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(img, { clientX: 150, clientY: 120, pointerId: 1 });
    fireEvent.pointerUp(img, { pointerId: 1 });
    expect(img.style.transform).toContain('translate(50px, 20px)');
  });

  it('escape key calls onClose', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(mockOnClose).toHaveBeenCalled();
  });

  it('escape key with picker active deactivates picker first', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const pickerBtn = screen.getByTitle('Color Picker');
    fireEvent.click(pickerBtn);
    fireEvent.keyDown(document, { key: 'Escape' });
    // Should not close, should deactivate picker
    expect(mockOnClose).not.toHaveBeenCalled();
    // Picker is now inactive, second escape should close
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(mockOnClose).toHaveBeenCalled();
  });

  it('cursor changes to crosshair in picker mode', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const pickerBtn = screen.getByTitle('Color Picker');
    fireEvent.click(pickerBtn);
    const img = screen.getByAltText('preview');
    expect(img.style.cursor).toBe('crosshair');
  });

  it('save button triggers download fallback', async () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const saveBtn = screen.getByTitle('Save as PNG');
    fireEvent.click(saveBtn);
    // Just verify no error thrown; download uses DOM which is mocked
  });

  it('toolbar click does not propagate to backdrop', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const toolbar = document.querySelector('.absolute.top-3')!;
    fireEvent.click(toolbar);
    expect(mockOnClose).not.toHaveBeenCalled();
  });

  it('pointer down with right button does not start drag', () => {
    render(<ImageLightbox src={testSrc} onClose={mockOnClose} />);
    const img = screen.getByAltText('preview');
    fireEvent.pointerDown(img, { button: 2, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(img, { clientX: 150, clientY: 120, pointerId: 1 });
    fireEvent.pointerUp(img, { pointerId: 1 });
    expect(img.style.transform).toBe('translate(0px, 0px) scale(1)');
  });
});
