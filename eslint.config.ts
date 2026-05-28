import pleaseai from '@pleaseai/eslint-config'

export default pleaseai({
  typescript: {
    tsconfigPath: 'tsconfig.json',
  },
  ignores: [
    'dist',
    'node_modules',
    '.csp',
  ],
})
