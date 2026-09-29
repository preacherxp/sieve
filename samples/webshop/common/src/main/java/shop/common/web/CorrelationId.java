package shop.common.web;

public final class CorrelationId {
    public static final String HEADER = "X-Correlation-Id";
    public static final String CONTEXT_KEY = "correlationId";

    private CorrelationId() {
    }
}
