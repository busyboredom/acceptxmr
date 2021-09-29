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
    
    var paid = message.paid_amount;
    document.getElementById("acceptxmr-paid").innerHTML = picoToXMR(paid);

    var due = message.expected_amount;
    document.getElementById("acceptxmr-due").innerHTML = picoToXMR(due);

    var confirmations = 0
    if (message.paid_at != null) {
        confirmations = Math.max(message.current_block - message.paid_at + 1, 0);
    }
    document.getElementById("acceptxmr-confirmations").innerHTML = confirmations;

    var confirmationsRequired = message.confirmations_required;
    document.getElementById("acceptxmr-confirmations-required").innerHTML = confirmationsRequired;

    var expirationBlocks = message.expiration_block - message.current_block;
    var instructionString = "Loading...";
    var instructionClass = "acceptxmr-instruction";
    var newAddressBtnHidden = true;
    if (confirmations >= confirmationsRequired) {
        instructionString = "Paid! Thank you"
        socket.close();
    } else if (message.paid_at != null) {
        instructionString = "Paid! Waiting for Confirmation..."
    } else if (expirationBlocks > 2) {
        instructionString = "Send Monero to Address Below"
    } else if (expirationBlocks > 0) {
        instructionString = "Address Expiring Soon";
        instructionClass += " acceptxmr-warning";
        newAddressBtnHidden = false;
    } else {
        instructionString = "Address Expired!";
        newAddressBtnHidden = false;
        socket.close();
    }
    document.getElementById("acceptxmr-instruction").innerHTML = instructionString;
    document.getElementById("acceptxmr-instruction").classList = instructionClass;
    document.getElementById("acceptxmr-new-address-btn").hidden = newAddressBtnHidden;
    document.getElementById("acceptxmr-address-copy-btn").disabled = !newAddressBtnHidden;

    if (newAddressBtnHidden) {
        var address = message.address;
        document.getElementById("acceptxmr-address").innerHTML = address;
    
        var qr = qrcode(0, "M");
        qr.addData(address);
        qr.make();
        document.getElementById('acceptxmr-qrcode-container').innerHTML = qr.createSvgTag({scalable: true});
    } else {
        document.getElementById("acceptxmr-address").innerHTML = "Expiring or expired...";
        document.getElementById('acceptxmr-qrcode-container').innerHTML = "<svg viewBox=\"0 0 100 100\" src=\"\"></svg>";
    }

};

socket.onclose = function(event) {
    if (event.code === 1000) {
        console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
    } else {
        // Server process killed or network down.
        // Event.code is usually 1006 in this case.
        alert('Connection died.');
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