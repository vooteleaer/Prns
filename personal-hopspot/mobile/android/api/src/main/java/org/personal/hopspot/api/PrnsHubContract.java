package org.personal.hopspot.api;

/** Names a client needs to find and bind the Personal Hopspot hub. */
public final class PrnsHubContract {
    /** The contract version this library was built against; see {@link IPrnsHub#apiVersion()}. */
    public static final int API_VERSION = 1;

    public static final String HUB_PACKAGE = "org.personal.hopspot";
    public static final String ACTION_BIND_HUB = "org.personal.hopspot.action.BIND_PRNS_HUB";

    public static final String STATE_STOPPED = "stopped";
    public static final String STATE_STARTING = "starting";
    public static final String STATE_RUNNING = "running";
    public static final String STATE_FAILED = "failed";

    private PrnsHubContract() {}
}
