"use strict";

// expo-sharing owns a fixed internal Xcode target and uses that internal name as the extension's
// visible name. Brand only CFBundleDisplayName; leave the target name, bundle id, signing team, and
// provisioning inputs untouched.
//
// Register this before expo-sharing. Expo applies same-phase mods in reverse registration order,
// so the generated target and plist exist by the time these mods execute.
const fs = require("node:fs");
const path = require("node:path");
const plist = require("plist");
const { withDangerousMod, withXcodeProject } = require("expo/config-plugins");

const TARGET_NAME = "expo-sharing-extension";

function entries(section) {
  return Object.entries(section ?? {}).filter(([key]) => !key.endsWith("_comment"));
}

function findNamed(section, name) {
  return entries(section).find(([, value]) => value?.name === name)?.[1] ?? null;
}

function withExtensionPlist(config, displayName) {
  return withDangerousMod(config, [
    "ios",
    (current) => {
      const file = path.join(
        current.modRequest.platformProjectRoot,
        TARGET_NAME,
        "Info.plist",
      );
      if (!fs.existsSync(file)) {
        throw new Error(`${file} was not generated; keep this plugin before expo-sharing.`);
      }
      const document = plist.parse(fs.readFileSync(file, "utf8"));
      document.CFBundleDisplayName = displayName;
      fs.writeFileSync(file, plist.build(document));
      return current;
    },
  ]);
}

function withExtensionBuildSettings(config, displayName) {
  return withXcodeProject(config, (current) => {
    const objects = current.modResults.hash.project.objects;
    const target = findNamed(objects.PBXNativeTarget, TARGET_NAME);
    if (!target) throw new Error(`Xcode target ${TARGET_NAME} was not generated.`);

    const configurationList = entries(objects.XCConfigurationList)
      .find(([key]) => key === target.buildConfigurationList)?.[1];
    if (!configurationList) {
      throw new Error(`Build configurations for ${TARGET_NAME} were not generated.`);
    }

    const configurations = objects.XCBuildConfiguration ?? {};
    for (const reference of configurationList.buildConfigurations ?? []) {
      const buildConfiguration = configurations[reference.value];
      if (buildConfiguration?.buildSettings) {
        buildConfiguration.buildSettings.INFOPLIST_KEY_CFBundleDisplayName =
          JSON.stringify(displayName);
      }
    }
    return current;
  });
}

module.exports = function withShareExtensionDisplayName(config) {
  const displayName = config.name || "Forge";
  return withExtensionBuildSettings(
    withExtensionPlist(config, displayName),
    displayName,
  );
};
