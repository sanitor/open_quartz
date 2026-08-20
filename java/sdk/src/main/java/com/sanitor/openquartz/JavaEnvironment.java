package com.sanitor.openquartz;

import java.util.Objects;

public final class JavaEnvironment {
    private final EnvironmentBridge bridge;

    public JavaEnvironment(EnvironmentBridge bridge) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
    }

    public EnvironmentBridge.Capabilities capabilities() {
        return bridge.capabilities();
    }

    public JavaEnvironment resourceResolver(EnvironmentBridge.ResourceResolver resolver) {
        bridge.setResourceResolver(Objects.requireNonNull(resolver, "resolver"));
        return this;
    }

    public JavaEnvironment inferenceProvider(EnvironmentBridge.InferenceProvider provider) {
        bridge.setInferenceProvider(Objects.requireNonNull(provider, "provider"));
        return this;
    }

    public JavaEnvironment presentationProvider(EnvironmentBridge.PresentationProvider provider) {
        bridge.setPresentationProvider(Objects.requireNonNull(provider, "provider"));
        return this;
    }
}
