package shop.catalog;

import static org.springframework.web.reactive.function.server.RouterFunctions.route;

import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.web.reactive.function.server.RouterFunction;
import org.springframework.web.reactive.function.server.ServerResponse;

@Configuration(proxyBeanMethods = false)
public class CatalogRouter {
    @Bean
    public RouterFunction<ServerResponse> catalogRoutes(CatalogHandler handler) {
        return route()
            .GET("/products/{sku}", handler::get)
            .GET("/products", handler::list)
            .build();
    }
}
