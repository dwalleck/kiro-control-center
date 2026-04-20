export type Tab = "Browse" | "Installed" | "Marketplaces" | "Kiro Settings";

export interface SettingCategory {
  key: string;
  label: string;
  count: number;
}
