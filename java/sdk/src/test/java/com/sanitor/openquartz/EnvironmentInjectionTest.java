package com.sanitor.openquartz;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicReference;

public final class EnvironmentInjectionTest {
    public static void main(String[] args) {
        FakeBridge fake = new FakeBridge();
        JniBridge bridge = new JniBridge(fake);
        AtomicReference<String> resolved = new AtomicReference<>();
        new JavaEnvironment(bridge)
            .resourceResolver(id -> {
                resolved.set(id);
                return CompletableFuture.completedFuture(new byte[] { 1, 2, 3 });
            })
            .inferenceProvider((node, task) -> CompletableFuture.completedFuture("{}"))
            .presentationProvider((node, dataUrl) -> resolved.set(node + ":" + dataUrl));

        assert bridge.capabilities().inference();
        assert bridge.resolveResource("image").toCompletableFuture().join().length == 3;
        assert resolved.get().equals("image");
        assert bridge.executeInference("onnx", "{}").toCompletableFuture().join().equals("{}");
        bridge.present("renderer", "data:image/png;base64,test");
        assert resolved.get().startsWith("renderer:data:image");
        System.out.println("Java environment injection contract passed");
    }
}
