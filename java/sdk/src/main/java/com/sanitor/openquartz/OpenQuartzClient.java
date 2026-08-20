package com.sanitor.openquartz;

import java.util.Objects;

public class OpenQuartzClient implements AutoCloseable {
    private final InternalBridge bridge;
    private long handle;
    private boolean closed;

    public OpenQuartzClient() {
        this(new NativeBridge());
    }

    public OpenQuartzClient(InternalBridge bridge) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
        this.handle = bridge.createClient();
    }

    public Project createProject(String name) {
        ensureOpen();
        return new Project(bridge, bridge.createProject(handle, name));
    }

    public Project openProject(String projectJson) {
        ensureOpen();
        return new Project(bridge, bridge.openProject(handle, projectJson));
    }

    public String sdkVersion() {
        ensureOpen();
        return bridge.sdkVersion();
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releaseClient(handle);
        handle = 0;
    }

    private void ensureOpen() {
        if (closed) throw new IllegalStateException("OpenQuartzClient is closed");
    }
}
