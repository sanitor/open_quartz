import { catalogSnapshot } from '../sdk/catalog';
import type { OnnxTask } from '../sdk/catalog';

export type { OnnxTask };

export interface CatalogEntry {
  id: string;
  label: string;
  category: string;
  icon?: string;
  taskLabel: string;
}

const snapshot = catalogSnapshot();

export const ONNX_CATALOG: Record<string, CatalogEntry> = Object.fromEntries(
  snapshot.onnxModels.map((entry) => [
    entry.id,
    {
      id: entry.id,
      label: entry.label,
      category: entry.category,
      taskLabel: entry.task,
    },
  ]),
);

export const CATALOG_CATEGORIES: string[] = snapshot.onnxCategories;
