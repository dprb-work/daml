import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import org.springframework.scheduling.support.CronExpression;

// Independently authored adapter: no upstream implementation or test text copied.
public class CronOracle {
    public static void main(String[] args) throws Exception {
        var input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        for (String line; (line = input.readLine()) != null;) {
            String expression = new String(Base64.getDecoder().decode(line), StandardCharsets.UTF_8);
            try {
                var parsed = CronExpression.parse(expression);
                System.out.println("OK\t" + Base64.getEncoder().encodeToString(parsed.toString().getBytes(StandardCharsets.UTF_8)));
            } catch (IllegalArgumentException failure) {
                System.out.println("ERR\t" + Base64.getEncoder().encodeToString(failure.getMessage().getBytes(StandardCharsets.UTF_8)));
            }
        }
    }
}
