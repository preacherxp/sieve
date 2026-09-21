package example;
public class CheckoutService { private final PaymentGateway gateway; public CheckoutService(PaymentGateway gateway) { this.gateway= gateway; } public String checkout(int net) { return gateway.charge(new PriceCalculator().total(net)); } }
