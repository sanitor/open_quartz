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
    captureScreenshot: null as ((rendererId: string) => string | null) | null,

    setSelectedNode: (id: string | null) => set((state) => { state.selectedNodeId = id; }),
    setActiveRenderer: (id: string | null) => set((state) => { state.activeRendererId = id; }),
    setOutputPreview: (nodeId: string, dataUrl: string) => set((state) => { state.outputPreviews[nodeId] = dataUrl; }),
    setOutputData: (nodeId: string, data: unknown) => set((state) => { state.outputData[nodeId] = data; }),
    clearOutputPreviews: () => set((state) => { state.outputPreviews = {}; state.outputData = {}; }),
    setNodeError: (nodeId: string, error: string | null) => set((state) => {
      if (error === null) {
        delete state.nodeErrors[nodeId];
      } else {
        state.nodeErrors[nodeId] = error;
      }
    }),
    clearNodeErrors: () => set((state) => { state.nodeErrors = {}; }),
    setCaptureScreenshot: (fn: ((rendererId: string) => string | null) | null) => set((state) => {
      state.captureScreenshot = fn as never;
    }),
  };
}
