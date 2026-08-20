package com.sanitor.openquartz;

import java.util.concurrent.CompletionStage;

public final class ErrorMapping {
    private ErrorMapping() {}

    public static SdkException fromRustJson(String json) {
        String code = field(json, "code");
        String message = field(json, "message");
        return new SdkException(code, message, field(json, "nodeId"), field(json, "details"));
    }

    private static String field(String json, String key) {
        String marker = "\"" + key + "\":\"";
        int start = json.indexOf(marker);
        if (start < 0) return null;
        start += marker.length();
        int end = json.indexOf('"', start);
        return end < 0 ? null : json.substring(start, end);
    }

    public static final class SdkException extends RuntimeException {
        private final String code;
        private final String nodeId;
        private final String details;

        SdkException(String code, String message, String nodeId, String details) {
            super(message == null ? "Rust SDK operation failed" : message);
            this.code = code;
            this.nodeId = nodeId;
            this.details = details;
        }

        public String code() { return code; }
        public String nodeId() { return nodeId; }
        public String details() { return details; }
    }
}
