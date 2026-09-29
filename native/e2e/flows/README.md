# Maestro flow index (Wave 33 lane 1).
#
# These flows run against an iOS Simulator dev/release build of the app
# (appId org.marula.app) pointed at a booted e2e stack. They cover the
# deterministic surfaces the simulator can exercise: navigation, auth, the
# scan injector, deep links, and the offline queue toggle. The camera,
# haptics, and background push legs are TestFlight-checklist items (see
# native/README.md "Operator gaps").
#
# Run:
#   maestro test native/e2e/flows
# The dev-menu airplane toggle and the scan injector exist so these flows
# need no OS-level network tricks and no camera.
