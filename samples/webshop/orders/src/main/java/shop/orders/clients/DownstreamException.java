package shop.orders.clients;

import org.springframework.http.HttpStatusCode;

/** A downstream service answered with an error the order flow maps to its own response. */
public class DownstreamException extends RuntimeException {
    private final String service;
    private final HttpStatusCode status;

    public DownstreamException(String service, HttpStatusCode status) {
        super(service + " answered " + status.value());
        this.service = service;
        this.status = status;
    }

    public String service() {
        return service;
    }

    public HttpStatusCode status() {
        return status;
    }
}
