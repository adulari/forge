# TypeScript config recovery reference

The fixture has an invalid TypeScript configuration, broken package entry point, and unsafe shallow
merge. The live run recovered from a rejected atomic multi-file edit. The reference passes strict
compilation and all secure deep-merge tests:

```bash
cd reference
npm test
npm run lint
```
