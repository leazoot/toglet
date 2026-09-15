import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  {
    ignores: ["dist/**", "src-tauri/**", "spikes/**", "design/**", "node_modules/**"],
  },

  // Config files are plain ESM and are deliberately outside the type-checked project,
  // so type-aware rules must not be applied to them.
  {
    files: ["**/*.js"],
    extends: [js.configs.recommended],
    languageOptions: {
      globals: globals.node,
      ecmaVersion: "latest",
      sourceType: "module",
    },
  },

  // The example phone page is a plain browser script: basic rules only, with browser globals.
  {
    files: ["examples/pwa/**/*.js"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.serviceworker },
    },
  },

  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      tseslint.configs.strictTypeChecked,
      tseslint.configs.stylisticTypeChecked,
    ],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // Type problems must be fixed, not silenced.
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/ban-ts-comment": "error",
      // No debug output in committed code.
      "no-console": "error",
      "no-debugger": "error",
    },
  },

  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": reactHooks, "react-refresh": reactRefresh },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // `invoke` and `listen` may only be called from src/ipc/. Enforced mechanically so the
      // boundary cannot erode as the interface grows.
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/api", "@tauri-apps/api/*", "@tauri-apps/plugin-*"],
              message: "Tauri IPC is only allowed in src/ipc/.",
            },
          ],
        },
      ],
    },
  },

  {
    files: ["src/ipc/**/*.{ts,tsx}"],
    rules: { "no-restricted-imports": "off" },
  },

  // vite.config.ts is type-checked through tsconfig.node.json, which the ESLint project service
  // does not load, so only syntactic rules apply to it here.
  {
    files: ["vite.config.ts"],
    extends: [tseslint.configs.disableTypeChecked],
    languageOptions: {
      globals: globals.node,
      parserOptions: { projectService: false, project: false },
    },
  },

  // Must stay last: turns off stylistic rules that would fight Prettier.
  prettier,
);
