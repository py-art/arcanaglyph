// entities/model/api.ts
//
// Тонкие invoke-обёртки для backend-команд, относящихся к моделям.
// Тип возвращаемых значений сейчас намеренно `any` где backend-схема
// богаче нашего entities/model/types.ts (например, поля `installed`,
// `path`, `description`, `download_url`, `default_filename`,
// `transcriber_type`). Полная типизация — отдельной задачей.

import { invoke } from '../../shared/lib/tauri';

export interface ModelDescriptor {
  id: string;
  display_name: string;
  description: string;
  size: string;
  available?: boolean;
  installed?: boolean;
  path?: string;
  download_url: string;
  default_filename: string;
  transcriber_type: string;
}

/** Все модели из реестра + их статус (installed / available). */
export async function getModels(): Promise<ModelDescriptor[]> {
  return (await invoke<ModelDescriptor[]>('get_models')) || [];
}

/** Список движков, скомпилированных в текущей сборке (cargo features). */
export async function getCompiledEngines(): Promise<string[]> {
  return (await invoke<string[]>('get_compiled_engines')) || [];
}

/** Скачать модель по id; backend сам пишет config-путь. */
export async function downloadModel(modelId: string, url: string, destDir: string): Promise<void> {
  await invoke('download_model', { modelId, url, destDir });
}

/** Удалить файлы модели физически с диска. */
export async function deleteModel(modelId: string, path: string): Promise<void> {
  await invoke('delete_model', { modelId, path });
}
