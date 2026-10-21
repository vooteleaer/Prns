package org.personal.hopspot.api;

/**
 * Snapshot of the hub's engine. Fields may only be appended; a client built against an older
 * contract reads the prefix it knows, and a newer field left unset keeps its default.
 */
parcelable HubStatus {
    /** "stopped", "starting", "running", or "failed". */
    String state;
    boolean running;
    int lastFailureCode;
    String lastFailure;
    /** Lowercase hex, or null while the engine is not running. */
    String nodeIdentityHashHex;
    String deliveryDestinationHex;
    String nodePageDestinationHex;
    long runtimeUptimeMs;
    int interfaceCount;
    int onlineInterfaceCount;
    /** Shared-instance clients, over both loopback TCP and this Binder interface. */
    int localClientCount;
    int routeCount;
    int linkCount;
    int transportedLinkCount;
    long rxBytes;
    long txBytes;
    long rxBps;
    long txBps;
}
