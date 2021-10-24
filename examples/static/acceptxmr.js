let socket = new WebSocket("ws://localhost:8080/ws/");

socket.onmessage = function (event) {
    var message = JSON.parse(event.data);

    // Show paid/due.
    document.getElementById("acceptxmr-paid").innerHTML = picoToXMR(message.amount_paid);
    document.getElementById("acceptxmr-due").innerHTML = picoToXMR(message.amount_requested);

    // Show confirmations/required.
    var confirmations = Math.max(0, message.confirmations);
    document.getElementById("acceptxmr-confirmations").innerHTML = confirmations;
    document.getElementById("acceptxmr-confirmations-required").innerHTML = message.confirmations_required;

    // Show instructive text depending on payment state.
    var instructionString = "Loading...";
    var instructionClass = "acceptxmr-instruction";
    var newAddressBtnHidden = true;
    if (confirmations >= message.confirmations_required) {
        instructionString = "Paid! Thank you"
        socket.close();
    } else if (message.amount_paid > message.amount_requested) {
        instructionString = "Paid! Waiting for Confirmation..."
    } else if (message.expiration_in > 2) {
        instructionString = "Send Monero to Address Below"
    } else if (message.expiration_in > 0) {
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

    // Hide address if nearing expiration.
    document.getElementById("acceptxmr-new-address-btn").hidden = newAddressBtnHidden;
    document.getElementById("acceptxmr-address-copy-btn").disabled = !newAddressBtnHidden;
    if (newAddressBtnHidden) {
        var address = message.address;
        document.getElementById("acceptxmr-address").innerHTML = address;

        var qr = qrcode(0, "M");
        qr.addData(address);
        qr.make();
        document.getElementById('acceptxmr-qrcode-container').innerHTML = qr.createSvgTag({ scalable: true });
    } else {
        document.getElementById("acceptxmr-address").innerHTML = "Expiring or expired...";
        document.getElementById('acceptxmr-qrcode-container').innerHTML = "<svg viewBox=\"0 0 100 100\" src=\"\"></svg>";
    }

};

// If the websocket closes cleanly, log it. Otherwise, alert the user.
socket.onclose = function (event) {
    if (event.code === 1000) {
        console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
    } else {
        // Server process killed or network down.
        // Event.code is usually 1006 in this case.
        alert('Connection died. If you have made your payment already, rest assured that it will still be processed.');
    }
};

// Convert from piconeros to monero.
function picoToXMR(amount) {
    let divisor = 1_000_000_000_000;
    let xmr = Math.floor(amount / divisor) + amount % divisor / divisor;
    return new Intl.NumberFormat(undefined, { maximumSignificantDigits: 20 }).format(xmr);
}

// Make the copy button work.
function copyPaymentAddress() {
    // Get the text field
    var copyText = document.getElementById("acceptxmr-address");

    // Copy the text inside the text field
    navigator.clipboard.writeText(copyText.innerHTML);

    // Provide feedback
    document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copied!";
    setTimeout(function () {
        document.getElementById("acceptxmr-address-copy-btn").innerHTML = "Copy";
    }, 1000);
}

socket.onerror = function (error) {
    alert(`[error] ${error.message}`);
};