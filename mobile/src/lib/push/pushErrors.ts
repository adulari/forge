// Platform-agnostic on purpose: every push implementation (push.ts / push.web.ts / push.ios.ts)
// and both toggle UIs import this same module, so it must not be part of the Metro
// platform-resolved trio beside them.
//
// The toggles already had two channels — a resolved non-"subscribed" state meant "the user
// declined permission", a thrown error meant "the network call failed". OS-level registration
// failure fits neither: permission is granted, the server was never reached. It used to collapse
// into the first channel, so the app told you to check a permission you had already given. That is
// what made a missing `aps-environment` entitlement undiagnosable from the device.
export class PushRegistrationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PushRegistrationError";
  }
}
