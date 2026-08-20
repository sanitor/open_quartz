package com.sanitor.openquartz;

import java.util.Objects;

public final class Project implements AutoCloseable {
    private final InternalBridge bridge;
    private long handle;
    private boolean closed;

    Project(InternalBridge bridge, long handle) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
        this.handle = handle;
    }

    public String name() {
        ensureOpen();
        return bridge.projectName(handle);
    }

    public void setName(String name) {
        ensureOpen();
        bridge.setProjectName(handle, Objects.requireNonNull(name, "name"));
    }

    public Graph graph() {
        ensureOpen();
        return new Graph(bridge, bridge.projectGraph(handle));
    }

    public Player createPlayer() {
        ensureOpen();
        return new Player(bridge, bridge.createPlayer(handle));
    }

    public String toJson() {
        ensureOpen();
        return bridge.projectJson(handle);
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releaseProject(handle);
        handle = 0;
    }

    private void ensureOpen() {
        if (closed) throw new IllegalStateException("Project is closed");
    }
}
