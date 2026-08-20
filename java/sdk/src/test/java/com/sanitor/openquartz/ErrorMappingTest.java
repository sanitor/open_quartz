package com.sanitor.openquartz;

public final class ErrorMappingTest {
    public static void main(String[] args) {
        ErrorMapping.SdkException error = ErrorMapping.fromRustJson(
            "{\"code\":\"unknown-node\",\"message\":\"Missing\",\"nodeId\":\"n1\"}"
        );
        assert error.code().equals("unknown-node");
        assert error.getMessage().equals("Missing");
        assert error.nodeId().equals("n1");
        System.out.println("Java error mapping contract passed");
    }
}
