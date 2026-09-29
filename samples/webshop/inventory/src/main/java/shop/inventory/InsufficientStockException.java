package shop.inventory;

import org.springframework.http.HttpStatus;
import org.springframework.web.server.ResponseStatusException;

public class InsufficientStockException extends ResponseStatusException {
    public InsufficientStockException(String sku, int quantity) {
        super(HttpStatus.CONFLICT, "Cannot reserve " + quantity + " x " + sku);
    }
}
