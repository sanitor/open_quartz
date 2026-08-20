import type { GraphState } from './index';

export function uiSlice(
  set: (fn: (state: GraphState) => void) => void,
  _get: () => GraphState,
) {
  return {
    selectedNodeId: null as string | null,
    activeRendererId: null as string | null,
    outputPreviews: {} as Record<string, string>,
    outputData: {} as Record<string, unknown>,
    nodeErrors: {} as Record<string, string>,
    rendererFps: {} as Record<string, number>,
    rendererCadence: {},
    rendererStreamActive: {} as Record<string, boolean>,

    setSelectedNode: (id: string | null) => set((state) => { state.selectedNodeId = id; }),
    setActiveRenderer: (id: string | null) => set((state) => { state.activeRendererId = id; }),
    setOutputPreview: (nodeId: string, dataUrl: string) => set((state) => { state.outputPreviews[nodeId] = dataUrl; }),
    setOutputData: (nodeId: string, data: unknown) => set((state) => { state.outputData[nodeId] = data; }),
    clearOutputPreviews: () => set((state) => { state.outputPreviews = {}; state.outputData = {}; }),
    setRendererFps: (nodeId: string, fps: number) => set((state) => { state.rendererFps[nodeId] = fps; }),
    setRendererCadence: (nodeId: string, metrics: GraphState['rendererCadence'][string]) => set((state) => {
      state.rendererCadence[nodeId] = metrics;
    }),
    setRendererStreamActive: (nodeId: string, active: boolean) => set((state) => {
      if (active) state.rendererStreamActive[nodeId] = true;
      else delete state.rendererStreamActive[nodeId];
    }),
    clearRendererFps: () => set((state) => { state.rendererFps = {}; state.rendererCadence = {}; }),
    setNodeError: (nodeId: string, error: string | null) => set((state) => {
      if (error === null) {
        delete state.nodeErrors[nodeId];
      } else {
        state.nodeErrors[nodeId] = error;
      }
    }),
    clearNodeErrors: () => set((state) => { state.nodeErrors = {}; }),
  };
}
