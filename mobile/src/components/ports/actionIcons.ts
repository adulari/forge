import React from "react";
import { Bug, Database, FlaskConical, Globe, Hammer, Package, Play, RefreshCw, Rocket, Server, Terminal, Wrench, Zap } from "lucide-react-native";

import type { EmptyStateIcon } from "../ds/EmptyState";

const ICONS: Record<string, EmptyStateIcon> = {
  play: Play,
  run: Play,
  dev: Play,
  start: Play,
  terminal: Terminal,
  shell: Terminal,
  server: Server,
  serve: Server,
  web: Globe,
  globe: Globe,
  browser: Globe,
  build: Hammer,
  hammer: Hammer,
  test: FlaskConical,
  flask: FlaskConical,
  bug: Bug,
  debug: Bug,
  package: Package,
  install: Package,
  database: Database,
  db: Database,
  deploy: Rocket,
  rocket: Rocket,
  lint: Wrench,
  wrench: Wrench,
  tool: Wrench,
  watch: RefreshCw,
  refresh: RefreshCw,
  zap: Zap,
  fast: Zap,
};

/** The glyph for an action's `icon` word; unknown or missing words fall back to Play. */
export function actionIconFor(word: string | null | undefined): EmptyStateIcon {
  return (word && ICONS[word.trim().toLowerCase()]) || Play;
}

export function ActionGlyph({ word, size, color }: { word: string | null | undefined; size: number; color: string }) {
  return React.createElement(actionIconFor(word), { size, color, strokeWidth: 1.75 });
}
