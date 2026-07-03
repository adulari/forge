import React from "react";
import { C, FONT } from "../theme";

export const TerminalWindow: React.FC<{
  width: number;
  height: number;
  title?: string;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ width, height, title = "forge", children, style }) => {
  return (
    <div
      style={{
        width,
        height,
        background: C.base,
        border: `1px solid ${C.surface}`,
        borderRadius: 14,
        overflow: "hidden",
        boxShadow: "0 40px 120px rgba(0,0,0,0.55), 0 0 0 1px rgba(0,0,0,0.4)",
        display: "flex",
        flexDirection: "column",
        ...style,
      }}
    >
      <div
        style={{
          height: 40,
          background: C.mantle,
          display: "flex",
          alignItems: "center",
          paddingLeft: 18,
          gap: 9,
          borderBottom: `1px solid ${C.crust}`,
          flexShrink: 0,
        }}
      >
        <div style={{ width: 13, height: 13, borderRadius: "50%", background: "#f38ba8" }} />
        <div style={{ width: 13, height: 13, borderRadius: "50%", background: "#f9e2af" }} />
        <div style={{ width: 13, height: 13, borderRadius: "50%", background: "#a6e3a1" }} />
        <div style={{ flex: 1, textAlign: "center", fontFamily: FONT, fontSize: 15, color: C.muted, marginRight: 60 }}>
          {title}
        </div>
      </div>
      <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>{children}</div>
    </div>
  );
};
