import type { GraphState } from './index';

export function projectSlice(
  set: (fn: (state: GraphState) => void) => void,
  _get: () => GraphState,
) {
  return {
    projectName: 'Untitled',
    savedFilePath: null as string | null,

    setProjectName: (name: string) => set((state) => { state.projectName = name; }),
    setSavedFilePath: (path: string | null) => set((state) => { state.savedFilePath = path; }),
  };
}
