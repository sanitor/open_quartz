package com.sanitor.openquartz;

public final class PublicProxyTest {
    public static void main(String[] args) {
        FakeBridge bridge = new FakeBridge();
        try (OpenQuartzClient client = new OpenQuartzClient(bridge)) {
            assert client.sdkVersion().equals("0.19.0");
            try (Project project = client.createProject("Demo"); Graph graph = project.graph()) {
                assert project.name().equals("Demo");
                assert graph.node("color").label().equals("Color");
                try (Player player = project.createPlayer(); Output output = player.output("renderer", "output")) {
                    player.play();
                    assert output.capture().toCompletableFuture().join().startsWith("data:image/png");
                    player.stop();
                }
            }
        }
        assert bridge.released;
        System.out.println("Java public proxy contract passed");
    }
}
