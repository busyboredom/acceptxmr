const BLOCK_TIME = 2;

function copyPaymentAddress() {
    /* Get the text field */
    var copyText = document.getElementById("acceptxmr-address");

    /* Copy the text inside the text field */
    navigator.clipboard.writeText(copyText.innerHTML);

    /* Provide feedback */
    document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copied!";
    setTimeout(function(){
        document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copy";  
    },1000);
} 

let socket = new WebSocket("ws://localhost:8080/ws/");

socket.onopen = function(e) {
};

socket.onmessage = function(event) {
    var message = JSON.parse(event.data);
    var address = message.address;
    document.getElementById("acceptxmr-address").innerHTML = address;

    var qr = qrcode(0, "M");
    qr.addData(address);
    qr.make();
    document.getElementById('acceptxmr-qrcode-container').innerHTML = qr.createSvgTag({scalable: true});

    var paid = message.paid_amount;
    document.getElementById("acceptxmr-paid").innerHTML = picoToXMR(paid);

    var due = message.expected_amount;
    document.getElementById("acceptxmr-due").innerHTML = picoToXMR(due);

    var confirmations = 0
    if (message.paid_at != null) {
        confirmations = Math.max(message.current_block - message.paid_at, 0);
    }
    document.getElementById("acceptxmr-confirmations").innerHTML = confirmations;

    var confirmationsRequired = message.confirmations_required;
    document.getElementById("acceptxmr-confirmations-required").innerHTML = confirmationsRequired;

    var currentBlock = message.current_block;
    document.getElementById("acceptxmr-current-block").innerHTML = currentBlock;

    var expirationBlocks = message.expiration_block - message.current_block;
    var expirationString = "";
    if (message.paid_at != null) {
        expirationString = "N/A"
    } else if (expirationBlocks >= 0) {
        var expirationTime = expirationBlocks * BLOCK_TIME;
        expirationString = expirationBlocks + " blocks (~";
        if (expirationTime < 60) {
            expirationString += expirationTime + " minutes)";
        } else {
            expirationString += Math.floor(expirationTime/60) + " hours)";
        }
        if (expirationBlocks <= message.confirmations_required) {
            expirationString.fontcolor("red");
        }
    } else {
        expirationString = "EXPIRED".fontcolor("red");
    }
    document.getElementById("acceptxmr-expiration-in").innerHTML = expirationString;
};

socket.onclose = function(event) {
    if (event.wasClean) {
        alert(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
    } else {
        // Server process killed or network down.
        // Event.code is usually 1006 in this case.
        alert('[close] Connection died');
    }
};

socket.onerror = function(error) {
    alert(`[error] ${error.message}`);
};

function picoToXMR(amount) {
    let divisor = 1_000_000_000_000;
    let xmr = Math.floor(amount / divisor) + amount % divisor / divisor;
    return new Intl.NumberFormat(undefined, { maximumSignificantDigits: 20 }).format(xmr);
}