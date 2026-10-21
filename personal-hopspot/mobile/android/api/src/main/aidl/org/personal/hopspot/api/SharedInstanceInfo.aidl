package org.personal.hopspot.api;

/**
 * How a stock Reticulum application (Sideband, Python RNS) joins the hub as a shared-instance
 * client over loopback TCP. Fields may only be appended.
 */
parcelable SharedInstanceInfo {
    int localPort;
    int rpcPort;
    /** Lowercase hex RPC key, or null while the engine is not running. */
    String rpcKeyHex;
    /** A complete Reticulum config for joining as a shared-instance client, or null. */
    String reticulumConfig;
}
