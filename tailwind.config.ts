import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        harbor: {
          rail: "#1E1F22",
          panel: "#2B2D31",
          controls: "#232428",
          floating: "#111214",
          modal: "#313338",
          hover: "#35373C",
          active: "#404249",
          border: "#1F2023",
          divider: "#35363C",
          handleBorder: "#3F4147",
          blurple: "#5865F2",
          blurpleHover: "#4752C4",
          success: "#23A55A",
          warning: "#F0B232",
          danger: "#DA373C",
          textPrimary: "#F2F3F5",
          textSecondary: "#B5BAC1",
          textMuted: "#949BA4",
        },
      },
      fontFamily: {
        sans: ["Inter", "Segoe UI Variable", "Segoe UI", "sans-serif"],
      },
      transitionTimingFunction: {
        discord: "cubic-bezier(0.2, 0, 0, 1)",
      },
    },
  },
  plugins: [],
};

export default config;
