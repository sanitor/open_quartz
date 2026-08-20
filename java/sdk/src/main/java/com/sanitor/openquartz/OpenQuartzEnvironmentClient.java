package com.sanitor.openquartz;

import java.util.Objects;

public final class OpenQuartzEnvironmentClient extends OpenQuartzClient {
    private final JavaEnvironment environment;

    public OpenQuartzEnvironmentClient(EnvironmentBridge bridge) {
        super(bridge);
        this.environment = new JavaEnvironment(Objects.requireNonNull(bridge, "bridge"));
    }

    public JavaEnvironment environment() {
        return environment;
    }
}
