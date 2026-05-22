export type Tab = "Browse" | "Installed" | "Marketplaces" | "Agents" | "Kiro Settings";

export interface SettingCategory {
  key: string;
  label: string;
  count: number;
}
