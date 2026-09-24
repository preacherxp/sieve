package shop.orders;

import org.springframework.http.HttpStatus;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.ExceptionHandler;
import org.springframework.web.bind.annotation.RestControllerAdvice;
import shop.common.web.ApiError;
import shop.orders.clients.DownstreamException;

/** Maps downstream failures to this service's API: unknown products and oversells are the caller's problem. */
@RestControllerAdvice
public class OrderErrors {
    @ExceptionHandler(DownstreamException.class)
    public ResponseEntity<ApiError> downstream(DownstreamException error) {
        if (error.service().equals("catalog") && error.status().value() == 404) {
            return ResponseEntity.status(HttpStatus.NOT_FOUND).body(new ApiError("PRODUCT_NOT_FOUND", error.getMessage()));
        }
        if (error.service().equals("inventory") && error.status().value() == 409) {
            return ResponseEntity.status(HttpStatus.CONFLICT).body(new ApiError("OUT_OF_STOCK", error.getMessage()));
        }
        return ResponseEntity.status(HttpStatus.BAD_GATEWAY).body(new ApiError("DOWNSTREAM_FAILURE", error.getMessage()));
    }
}
