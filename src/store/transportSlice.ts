import type { GraphState } from './index';

export function transportSlice(
  set: (fn: (state: GraphState) => void) => void,
  _get: () => GraphState,
) {
  return {
    loopState: 'stopped' as const,
    fps: 0,
    currentTime: 0,
    currentFrame: 0,

    play: () => set((state) => { state.loopState = 'playing'; }),
    pause: () => set((state) => { state.loopState = 'paused'; }),
    resume: () => set((state) => { state.loopState = 'playing'; }),
    stop: () => set((state) => {
      state.loopState = 'stopped';
      state.fps = 0;
      state.currentTime = 0;
      state.currentFrame = 0;
    }),
    setFps: (fps: number) => set((state) => { state.fps = fps; }),
    setCurrentTime: (t: number) => set((state) => { state.currentTime = t; }),
    setCurrentFrame: (frame: number) => set((state) => { state.currentFrame = frame; }),
  };
}
