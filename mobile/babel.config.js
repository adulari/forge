module.exports = function (api) {
  api.cache(true);
  return {
    presets: ["babel-preset-expo"],
    // babel-preset-expo auto-detects react-native-worklets/react-native-reanimated
    // and injects the correct worklets babel plugin — no manual plugin entry needed.
  };
};
