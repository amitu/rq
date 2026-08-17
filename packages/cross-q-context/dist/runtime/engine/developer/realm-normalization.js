/**
 * Generic JSON roundtrip for VM-realm objects.
 *
 * Objects created inside vm.createContext belong to a different JS realm —
 * their prototypes don't match the host realm's Object.prototype, which
 * breaks structuredClone, Cap'n Web serialization, and other prototype-based
 * checks. JSON stringify→parse produces plain objects in the current realm.
 *
 * Falls back to String() for non-serializable values (circular refs, functions).
 */
export function normalizeFromVm(value) {
    try {
        return JSON.parse(JSON.stringify(value));
    }
    catch {
        return String(value);
    }
}
