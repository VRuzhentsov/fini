import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

export interface BackupManifest {
  format: string;
  version: number;
  app_version: string;
  exported_at: string;
  domains: string[];
  spaces: Array<{ id: string; name: string }>;
  counts: { spaces: number; quest_series: number; quests: number };
}

export interface BackupExportResult {
  path: string;
  manifest: BackupManifest;
}

export interface BackupSpaceMappingRequest {
  backup_space_id: string;
  backup_space_name: string;
}

export interface BackupConflict {
  entity_type: string;
  id: string;
  title: string;
  local_summary: string;
  backup_summary: string;
  local: unknown;
  backup: unknown;
}

export interface BackupImportPreflight {
  manifest: BackupManifest;
  required_space_mappings: BackupSpaceMappingRequest[];
  conflicts: BackupConflict[];
}

export interface BackupSpaceMappingInput {
  backup_space_id: string;
  mode: "create_new" | "use_existing";
  local_space_id?: string;
}

export interface BackupConflictResolutionInput {
  entity_type: string;
  id: string;
  resolution: "local" | "backup";
}

export interface BackupImportResult {
  imported: boolean;
  spaces: number;
  quest_series: number;
  quests: number;
}

export const useBackupStore = defineStore("backup", () => {
  function exportBackup(path: string, spaceIds: string[]): Promise<BackupExportResult> {
    return invoke<BackupExportResult>("backup_export", { path, spaceIds });
  }

  function preflightImport(path: string, mappings: BackupSpaceMappingInput[]): Promise<BackupImportPreflight> {
    return invoke<BackupImportPreflight>("backup_preflight_import", { path, mappings });
  }

  function applyImport(
    path: string,
    mappings: BackupSpaceMappingInput[],
    resolutions: BackupConflictResolutionInput[],
  ): Promise<BackupImportResult> {
    return invoke<BackupImportResult>("backup_apply_import", { path, mappings, resolutions });
  }

  return { exportBackup, preflightImport, applyImport };
});
