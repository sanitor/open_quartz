package com.sanitor.openquartz;

import java.util.concurrent.CompletionStage;

public final class Output implements AutoCloseable {
    private final InternalBridge bridge;
    private final long playerHandle;
    private long handle;
    private boolean closed;

    Output(InternalBridge bridge, long playerHandle, long handle) {
        this.bridge = bridge;
        this.playerHandle = playerHandle;
        this.handle = handle;
    }

    long handle() {
        if (closed) throw new IllegalStateException("Output is closed");
        return handle;
    }

    public String nodeId() { return bridge.outputNodeId(handle()); }
    public CompletionStage<String> capture() { return bridge.capture(playerHandle, handle()); }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releaseOutput(handle);
        handle = 0;
    }
}
