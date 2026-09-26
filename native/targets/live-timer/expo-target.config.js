/** @type {import('@bacons/apple-targets/app.plugin').ConfigFunction} */
module.exports = () => ({
  type: "widget",
  name: "live-timer",
  displayName: "Meridian Timer",
  // Live Activities need iOS 16.2 (ActivityKit stable).
  deploymentTarget: "16.2",
});
