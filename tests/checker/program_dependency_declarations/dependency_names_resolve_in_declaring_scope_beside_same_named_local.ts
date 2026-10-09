// @strict: true
// @filename: node_modules/core/index.d.ts
export type RulesConfig = Record<string, unknown>;
export interface Plugin {
  meta?: { name?: string };
  configs?: Record<string, LegacyConfigObject | ConfigObject | ConfigObject[]> | undefined;
}
export interface ConfigObject<Rules extends RulesConfig = RulesConfig> {
  name?: string;
  plugins?: Record<string, Plugin>;
  rules?: Partial<Rules>;
}
export interface LegacyConfigObject<Rules extends RulesConfig = RulesConfig, OverrideRules extends RulesConfig = Rules> {
  plugins?: string[];
  rules?: Partial<Rules>;
  overrides?: Partial<OverrideRules>[];
}
// @filename: node_modules/linter/index.d.ts
import type { Plugin as CorePlugin, ConfigObject, LegacyConfigObject, RulesConfig } from 'core';
export namespace ESLint {
  type Plugin = CorePlugin;
}
export namespace Linter {
  type RulesRecord = RulesConfig;
  type Config<Rules extends RulesRecord = RulesRecord> = ConfigObject<Rules>;
  type LegacyConfig<Rules extends RulesRecord = RulesRecord> = LegacyConfigObject<Rules>;
}
// @filename: src/index.ts
import type { ESLint, Linter } from 'linter';
declare const plugin: { configs: { flat: Linter.Config[]; legacy: Linter.LegacyConfig } };
export const accepted: ESLint.Plugin = plugin;
export const wrong: ESLint.Plugin = { configs: { flat: 1 } };
export interface Plugin { rules: 1 }
