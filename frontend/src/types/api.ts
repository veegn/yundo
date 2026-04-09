/**
 * Shared API types — mirrors the Rust `HistoryItem` struct serialized by the backend.
 * Import from here instead of redeclaring in each page component.
 */

export interface HistoryItem {
  slug: string;
  url: string;
  file_name: string;
  file_size: number;
  last_download_at: string;
  count_7d: number;
  score: number;
}
