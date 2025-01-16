const js = require('@eslint/js');
const tseslint = require('@typescript-eslint/eslint-plugin');
const prettier = require('eslint-config-prettier');

module.exports = [
  js.configs.recommended,
  tseslint.configs.recommended,
  prettier,
  {
    files: ['src/**/*.ts'],
    languageOptions: {
      sourceType: 'module',
      ecmaVersion: 'latest',
      globals: {
        node: true
      }
    },
    plugins: {
      '@typescript-eslint': tseslint
    },
    rules: {
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
      '@typescript-eslint/no-explicit-any': 'error',
      'no-console': 'error'
    }
  }
];